//! Markdown: rendering page bodies, and pulling the link graph out of them.
//!
//! Rendering is opt-in: the API serves raw markdown by default, because that is
//! what an agent can reason about and edit. HTML is for the browser.
//!
//! Links are extracted from comrak's AST rather than by scanning the source.
//! That is what makes `[[not a link]]` inside a code fence stay inside the code
//! fence — the parser has already decided what is code and what is prose, and a
//! regex over the raw text would have to relitigate it and get it wrong.

use std::ops::Range;

use comrak::nodes::{AstNode, NodeValue, Sourcepos};
use comrak::{Arena, Options};

use crate::slug::Slug;

/// Where a rendered link to a page points.
///
/// This is the browsable URL, not the API one: rendered HTML is for a human
/// reading the page, and `/api/pages/notes/a` would hand them JSON. The backend
/// serves this route itself — the SPA fallback in [`crate::api`] is what makes
/// it resolve — so this is a real URL on this origin rather than an assumption
/// about some other frontend.
pub const PAGE_URL_PREFIX: &str = "/pages/";

/// Render a markdown body to HTML.
///
/// `source` is the slug of the page being rendered, which decides what its
/// relative links mean. `None` renders as though the content sat at the wiki
/// root — the right answer for previewing a draft that has no slug yet.
///
/// Raw HTML in the source is **escaped, not passed through**. Rhizolog is
/// single-user and loopback-bound, so this is not guarding against a hostile
/// author — but page content arrives over an API that agents write to, and
/// rendering `<script>` from an indirect source into the dashboard is the kind
/// of thing that is very hard to notice and very easy to avoid. Turning this on
/// should be a deliberate, separate decision.
pub fn render(source: Option<&Slug>, markdown: &str) -> String {
    let options = options();
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, markdown, &options);

    resolve_page_links(base_of(source), root);

    let mut html = String::new();
    comrak::format_html(root, &options, &mut html).expect("writing into a String cannot fail");
    html
}

/// Point every link that names a page at its browsable URL.
///
/// comrak renders `[[notes/a]]` as `href="notes/a"`, which is *relative*: read
/// on `/pages/notes/b` the browser resolves it to `/pages/notes/notes/a`. Every
/// wikilink in a rendered body would land somewhere that does not exist, and it
/// would fail differently depending on how deeply nested the page reading it
/// was. Rewriting to a root-absolute URL is what makes rendered links work at
/// all.
///
/// External links are left exactly as written.
fn resolve_page_links<'a>(base: &str, root: &'a AstNode<'a>) {
    for node in root.descendants() {
        match &mut node.data.borrow_mut().value {
            NodeValue::WikiLink(wiki) => {
                // Wikilink targets are slugs from the wiki root, so they need
                // no resolution — only validation and encoding.
                if let Ok(slug) = Slug::parse(wiki.url.trim()) {
                    wiki.url = page_url(slug.as_str());
                }
            }
            NodeValue::Link(link) => {
                if let Some((target, LinkKind::Internal)) = classify(base, &link.url) {
                    link.url = page_url(&target);
                }
            }
            _ => {}
        }
    }
}

/// The browsable URL for a slug.
fn page_url(slug: &str) -> String {
    let mut url = String::from(PAGE_URL_PREFIX);

    for (index, segment) in slug.split('/').enumerate() {
        if index > 0 {
            url.push('/');
        }
        encode_segment(segment, &mut url);
    }

    url
}

/// Percent-encode one path segment.
///
/// Slugs may hold spaces, `#`, and `%`, none of which can go into a URL path
/// verbatim — a `#` would turn the rest of the slug into a fragment. Only the
/// unreserved set survives, which is a little stricter than the frontend's
/// `encodeURIComponent` but decodes to the same string.
fn encode_segment(segment: &str, out: &mut String) {
    use std::fmt::Write as _;

    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => {
                let _ = write!(out, "%{byte:02X}");
            }
        }
    }
}

/// The directory a page's relative links resolve against.
fn base_of(source: Option<&Slug>) -> &str {
    source.and_then(Slug::parent).unwrap_or("")
}

/// The parser configuration every reader of a page body shares.
///
/// Rendering, link extraction, word counting and the heading shift in
/// [`crate::compile`] all go through this, so all four agree about what is a
/// fence, a heading and a wikilink. Two of them disagreeing would be a page
/// whose links come from one reading and whose headings come from another.
pub fn parser_options() -> Options<'static> {
    options()
}

/// Fields are set individually rather than through a struct literal because
/// comrak's option types are `non_exhaustive` and have been renamed across
/// releases; touching only the fields we care about survives both.
fn options() -> Options<'static> {
    let mut options = Options::default();

    options.extension.strikethrough = true;
    options.extension.table = true;
    options.extension.autolink = true;
    options.extension.tasklist = true;
    options.extension.footnotes = true;

    // `[[slug]]` and `[[slug|display text]]`. Having comrak parse these rather
    // than scanning for them ourselves is what keeps wikilinks inside code
    // fences from being mistaken for real links.
    options.extension.wikilinks_title_after_pipe = true;

    // `options.render.unsafe_` is deliberately left at its default of false.
    // See the note on `render`.

    options
}

/// Where a link points.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LinkKind {
    /// `[[slug]]` — always a page, always absolute from the wiki root.
    Wiki,
    /// A markdown link that resolves to a page.
    Internal,
    /// A markdown link that leaves the wiki.
    External,
}

impl LinkKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wiki => "wiki",
            Self::Internal => "internal",
            Self::External => "external",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "wiki" => Some(Self::Wiki),
            "internal" => Some(Self::Internal),
            "external" => Some(Self::External),
            _ => None,
        }
    }

    /// Whether this link points at a page, and so belongs in the page graph.
    pub fn is_internal(self) -> bool {
        matches!(self, Self::Wiki | Self::Internal)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Link {
    /// A slug for [`LinkKind::Wiki`] and [`LinkKind::Internal`], a URL for
    /// [`LinkKind::External`].
    pub target: String,
    /// The link's text, when it says something other than the target.
    pub display: Option<String>,
    pub kind: LinkKind,
}

/// Pull every link out of a page body.
///
/// `source` is the slug of the page being read, needed to resolve relative
/// markdown links like `[async](../rust/async.md)`.
///
/// Links are returned in document order, deduplicated on (target, kind): a page
/// that links somewhere three times is one edge in the graph.
pub fn extract_links(source: &Slug, markdown: &str) -> Vec<Link> {
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, markdown, &options());
    let base = base_of(Some(source));

    let mut links: Vec<Link> = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for node in root.descendants() {
        let link = match &node.data.borrow().value {
            NodeValue::WikiLink(wiki) => {
                // Wikilink targets are slugs as written, from the wiki root.
                Slug::parse(wiki.url.trim())
                    .ok()
                    .map(|slug| build(slug.to_string(), text_of(node), LinkKind::Wiki))
            }
            NodeValue::Link(markdown_link) => classify(base, &markdown_link.url)
                .map(|(target, kind)| build(target, text_of(node), kind)),
            // An image is not a link to a page.
            _ => None,
        };

        if let Some(link) = link
            && seen.insert((link.target.clone(), link.kind))
        {
            links.push(link);
        }
    }

    links
}

/// How many words a page body holds.
///
/// The count an editor means, which is not the count `wc -w` gives: code is not
/// prose, and neither is a link's target or an image's alt text. Every rule here
/// is a consequence of that one sentence, and each is covered by a test, because
/// a number nobody can reproduce is a number to argue with.
///
/// Counted, from the AST rather than the source:
///
/// - Text in paragraphs, headings, list items, block quotes, tables and
///   footnote definitions.
/// - A link's **text**, never its target. `[the notes](notes/rust/async.md)` is
///   two words. A bare `[[notes/rust/async]]` is one, because the slug is what
///   the page displays.
///
/// Not counted: fenced and inline code, raw HTML, image alt text, frontmatter
/// (which is not in the body at all), and footnote *references*, which are
/// markers rather than words.
///
/// Raw HTML divides in a way worth knowing. A raw HTML **block** takes its
/// contents with it, since the renderer drops the whole thing and none of it
/// reaches the page. An **inline** tag does not: `<span>` is dropped and the
/// words it wraps are still rendered and still read, so they are still words.
///
/// A word is a whitespace-separated run holding at least one alphanumeric
/// character, so `--` and `|` in a table rule are not words and `it's` is one.
pub fn count_words(markdown: &str) -> u64 {
    readable_text(markdown)
        .split_whitespace()
        .filter(is_word)
        .count() as u64
}

/// The same words, as words.
///
/// What [`count_words`] counts, kept rather than tallied, for
/// [`crate::words::diff`] to compare two versions of a page with. Separate from
/// the counter rather than the counter being written over it, because counting
/// is what runs on every page of a twenty-thousand-page rebuild and it should
/// not start allocating a string per word to do it.
pub fn words(markdown: &str) -> Vec<String> {
    readable_text(markdown)
        .split_whitespace()
        .filter(is_word)
        .map(str::to_owned)
        .collect()
}

/// A word is a whitespace-separated run holding at least one alphanumeric
/// character, so `--` and `|` in a table rule are not words and `it's` is one.
fn is_word(run: &&str) -> bool {
    run.chars().any(char::is_alphanumeric)
}

/// Everything on the page that is read, as one string.
fn readable_text(markdown: &str) -> String {
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, markdown, &options());

    let mut text = String::new();
    collect_text(root, &mut text);
    text
}

/// What one node contributes to the readable text of a page.
///
/// The one place this wiki decides what counts as prose. Two readers walk the
/// AST for it and they must never disagree: [`count_words`] concatenates the
/// literals, and [`extract`] blanks out everything that is not one so that byte
/// offsets survive. A second copy of this match is where the two would quietly
/// come apart over whether a footnote is prose.
enum Role {
    /// A text literal. Its bytes are the prose.
    Literal,
    /// Neither this node nor anything under it is read.
    Skip,
    /// Not prose itself, but it keeps what is on either side of it apart.
    Break,
    /// Look at the children. `block` says whether this node separates as well.
    Descend { block: bool },
}

fn role(value: &NodeValue) -> Role {
    match value {
        NodeValue::Text(_) => Role::Literal,
        // Code is not prose. This is the whole reason both readers go through
        // the parser: the parser has already decided what is code, and a scan
        // over the source would have to relitigate it. It is the same argument
        // `extract_links` makes above.
        NodeValue::Code(_) | NodeValue::CodeBlock(_) => Role::Skip,
        // Dropped by the renderer, so it is not on the page to be read.
        NodeValue::HtmlInline(_) | NodeValue::HtmlBlock(_) => Role::Skip,
        // Alt text describes a picture rather than being part of the prose, and
        // the whole subtree goes with it.
        NodeValue::Image(_) => Role::Skip,
        // A marker, not a word.
        NodeValue::FootnoteReference(_) => Role::Skip,
        NodeValue::SoftBreak | NodeValue::LineBreak => Role::Break,
        other => Role::Descend {
            block: other.block(),
        },
    }
}

/// Gather the readable text of a subtree.
///
/// Literals are concatenated **verbatim**, with separators added only where the
/// source had one. That is load-bearing: comrak splits `un*believable*` into two
/// nodes and `hello *world*` into two nodes, and the difference between them is
/// the space inside the first `Text` literal. Joining every node with a space
/// would make the first two words; joining with nothing would make the second
/// one. Following the literals is the only version that gets both right.
///
/// Blocks and breaks contribute a newline, so the last word of one paragraph and
/// the first of the next do not run together into one.
fn collect_text<'a>(node: &'a AstNode<'a>, out: &mut String) {
    {
        let data = node.data.borrow();

        match role(&data.value) {
            Role::Literal => {
                if let NodeValue::Text(literal) = &data.value {
                    out.push_str(literal);
                }
                return;
            }
            Role::Skip => return,
            Role::Break => {
                out.push('\n');
                return;
            }
            Role::Descend { block } => {
                if block {
                    out.push('\n');
                }
            }
        }
    }

    for child in node.children() {
        collect_text(child, out);
    }
}

/// A page body with everything that is not prose blanked out.
///
/// The trick that makes [`crate::prose`] possible. A finding has to say where in
/// the source it fired, and the obvious approach, concatenating the text out of
/// the AST, throws that away: offsets into the concatenation are offsets into a
/// string nobody has. So instead of building a new string, this keeps the body's
/// own bytes and blanks everything that is not read: code, raw HTML, image alt
/// text, footnote markers, link targets, and every piece of markdown punctuation
/// in between. **Every offset into [`Extracted::text`] is an offset into the
/// body**, with no mapping in between to get wrong.
///
/// What is prose is decided by [`role`], which is the same decision
/// [`count_words`] follows.
///
/// It disagrees with [`count_words`] in one narrow place, and the difference is
/// worth stating: markup *inside* a word. `un*believable*` is one word to the
/// counter, which follows the literals, and two tokens here, because the `*`
/// between them is blanked and a token is a run of alphanumerics. Keeping the
/// offsets is worth that, and the tokenizer this feeds already splits on
/// punctuation everywhere else.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Extracted {
    /// The body, byte for byte, with every byte that is not prose replaced by a
    /// space. Newlines are kept as newlines, so a line and column read off this
    /// still name the same place in the source.
    pub text: String,
    /// The paragraphs, in source order, as spans of [`Extracted::text`].
    ///
    /// Rhythm is a property of a paragraph, so this is what a sentence-shaped
    /// rule measures over. Headings, table cells and list items are deliberately
    /// not in it: a list is uniform by construction, and a rule that fired on
    /// every bulleted list would be a rule nobody leaves switched on.
    pub paragraphs: Vec<Range<usize>>,
}

/// Blank out everything in a body that is not prose. See [`Extracted`].
pub fn extract(body: &str) -> Extracted {
    let arena = Arena::new();
    let root = comrak::parse_document(&arena, body, &options());

    let mut line_starts = Vec::new();
    let mut at = 0;
    for line in body.split_inclusive('\n') {
        line_starts.push(at);
        at += line.len();
    }

    // A byte for every byte of the source, so nothing has to be mapped later.
    let mut text: Vec<u8> = body
        .bytes()
        .map(|byte| if byte == b'\n' { b'\n' } else { b' ' })
        .collect();
    let mut paragraphs = Vec::new();

    blank(root, body, &line_starts, false, &mut text, &mut paragraphs);

    paragraphs.sort_by_key(|span| span.start);

    Extracted {
        // Every range copied in is a range of the source, and every byte left
        // behind is ASCII, so the result cannot be anything but valid UTF-8.
        text: String::from_utf8(text).expect("blanking preserves UTF-8"),
        paragraphs,
    }
}

/// Copy the prose of one subtree back over the blanked buffer.
///
/// `in_item` carries whether an ancestor is a list item, which is the one thing
/// a paragraph's own node cannot say and which decides whether it counts as
/// rhythm. Footnote definitions live at the end of comrak's tree rather than
/// where they were written, which is why the caller sorts.
fn blank<'a>(
    node: &'a AstNode<'a>,
    source: &str,
    line_starts: &[usize],
    in_item: bool,
    text: &mut [u8],
    paragraphs: &mut Vec<Range<usize>>,
) {
    let mut in_item = in_item;

    {
        let data = node.data.borrow();

        match role(&data.value) {
            Role::Literal => {
                if let Some(span) = span_of(line_starts, source, data.sourcepos)
                    && let Some(literal) = source.get(span.clone())
                {
                    text[span].copy_from_slice(literal.as_bytes());
                }
                return;
            }
            Role::Skip => return,
            // A soft break is already a newline in the buffer, and a hard break
            // is already whitespace. Neither needs anything written back.
            Role::Break => return,
            Role::Descend { .. } => match &data.value {
                NodeValue::Paragraph if !in_item => {
                    if let Some(span) = span_of(line_starts, source, data.sourcepos) {
                        paragraphs.push(span);
                    }
                }
                NodeValue::Item(_) | NodeValue::TaskItem(_) => in_item = true,
                _ => {}
            },
        }
    }

    for child in node.children() {
        blank(child, source, line_starts, in_item, text, paragraphs);
    }
}

/// Turn comrak's line and column pair into a byte range.
///
/// The columns are 1-based **byte** offsets within their line, which is what
/// makes this arithmetic rather than a character walk. `None` for anything that
/// does not land on a character boundary inside the source, so a node with a
/// sourcepos nobody filled in is skipped rather than panicking a request.
fn span_of(line_starts: &[usize], source: &str, pos: Sourcepos) -> Option<Range<usize>> {
    let start =
        line_starts.get(pos.start.line.checked_sub(1)?)? + pos.start.column.checked_sub(1)?;
    let end = line_starts.get(pos.end.line.checked_sub(1)?)? + pos.end.column;

    (start <= end && source.get(start..end).is_some()).then_some(start..end)
}

/// Drop display text that merely repeats the target.
///
/// A bare `[[notes/a]]` parses with `notes/a` as its label, but that is the
/// target restated, not a caption. `display` is meant to carry what the link
/// *says* when that differs, so callers can render it without having to
/// compare the two themselves.
fn build(target: String, display: Option<String>, kind: LinkKind) -> Link {
    Link {
        display: display.filter(|text| text != &target),
        target,
        kind,
    }
}

/// Decide what a markdown link target is, and normalise it if it is a page.
///
/// `base` is the directory relative targets resolve against — the source page's
/// parent, or `""` for a page at the wiki root. It is a plain path rather than a
/// [`Slug`] so that rendering a draft with no slug of its own can still resolve
/// links, without inventing a slug to stand in for one.
///
/// Returns `None` for things that are not page links at all: bare fragments,
/// and internal-looking targets that cannot be normalised into a valid slug.
/// Those are dropped rather than recorded as broken, because a "wanted page"
/// should be something you could go and create — `../../outside-the-wiki` is
/// not.
fn classify(base: &str, url: &str) -> Option<(String, LinkKind)> {
    let url = url.trim();
    if url.is_empty() || url.starts_with('#') {
        return None;
    }

    if has_scheme(url) || url.starts_with("//") {
        return Some((url.to_owned(), LinkKind::External));
    }

    // Drop any fragment or query before treating the rest as a path.
    let path = strip_md_extension(url.split(['#', '?']).next().unwrap_or(url));

    let resolved = if let Some(absolute) = path.strip_prefix('/') {
        normalize(absolute.split('/'))?
    } else {
        // Relative to the directory the source page lives in.
        normalize(base.split('/').chain(path.split('/')))?
    };

    Slug::parse(&resolved)
        .ok()
        .map(|slug| (slug.to_string(), LinkKind::Internal))
}

/// `https:`, `mailto:`, `C:` — anything shaped like a URL scheme.
fn has_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };

    !scheme.is_empty()
        && scheme.starts_with(|c: char| c.is_ascii_alphabetic())
        && scheme
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
}

fn strip_md_extension(path: &str) -> &str {
    let split = match path.len().checked_sub(3) {
        Some(split) => split,
        None => return path,
    };
    let (head, tail) = path.split_at(split);
    if tail.eq_ignore_ascii_case(".md") {
        head
    } else {
        path
    }
}

/// Collapse `.` and `..` segments. Returns `None` if the path climbs out of
/// the wiki root or ends up empty.
fn normalize<'a>(parts: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();

    for part in parts {
        match part {
            "" | "." => {}
            ".." => {
                segments.pop()?;
            }
            other => segments.push(other),
        }
    }

    (!segments.is_empty()).then(|| segments.join("/"))
}

/// The visible text of a link node.
fn text_of<'a>(node: &'a AstNode<'a>) -> Option<String> {
    let mut text = String::new();

    for child in node.descendants().skip(1) {
        match &child.data.borrow().value {
            NodeValue::Text(literal) => text.push_str(literal),
            NodeValue::Code(code) => text.push_str(&code.literal),
            _ => {}
        }
    }

    let text = text.trim().to_owned();
    (!text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Render as though the content had no slug of its own.
    fn html(markdown: &str) -> String {
        render(None, markdown)
    }

    /// Render as a page living at `source`.
    fn html_from(source: &str, markdown: &str) -> String {
        render(Some(&Slug::parse(source).expect("valid slug")), markdown)
    }

    #[test]
    fn renders_ordinary_markdown() {
        let html = html("# Heading\n\nSome *emphasis* and a [link](https://example.com).\n");

        assert!(html.contains("<h1>Heading</h1>"));
        assert!(html.contains("<em>emphasis</em>"));
        assert!(html.contains(r#"<a href="https://example.com">link</a>"#));
    }

    #[test]
    fn renders_gfm_extensions() {
        assert!(html("~~gone~~").contains("<del>gone</del>"));
        assert!(html("| a | b |\n|---|---|\n| 1 | 2 |\n").contains("<table>"));
        assert!(html("- [x] done\n").contains("checked"));
        assert!(html("Visit https://example.com today").contains("<a href="));
    }

    /// The whole point of leaving comrak's `unsafe_` option off.
    ///
    /// Note what comrak actually does here: with `unsafe_` off a raw HTML block
    /// is *dropped* and replaced by a comment, not escaped into visible text.
    /// The security property is the same, but the rendered page will not show
    /// the markup either, which is worth knowing before someone reports it as a
    /// bug.
    #[test]
    fn raw_html_is_not_passed_through() {
        let html = html("<script>alert('xss')</script>\n");

        assert!(
            !html.contains("<script"),
            "raw HTML was passed through: {html}"
        );
        assert!(
            html.contains("raw HTML omitted"),
            "expected comrak's omission marker, got {html}"
        );
    }

    #[test]
    fn inline_html_is_escaped_too() {
        let html = html("Text with <img src=x onerror=alert(1)> inline.\n");

        assert!(
            !html.contains("<img"),
            "inline HTML was passed through: {html}"
        );
    }

    /// A javascript: URL must not survive as a clickable link.
    #[test]
    fn dangerous_link_schemes_are_neutralised() {
        let html = html("[click me](javascript:alert(1))\n");

        assert!(
            !html.contains("href=\"javascript:"),
            "javascript: URL survived: {html}"
        );
    }

    #[test]
    fn an_empty_body_renders_to_nothing() {
        assert_eq!(html("").trim(), "");
    }

    // -------------------------------------------------- links in rendered HTML

    /// The bug this rewriting exists to prevent: read on `/pages/notes/b`, a
    /// relative `href="notes/a"` would resolve to `/pages/notes/notes/a`.
    #[test]
    fn rendered_page_links_are_root_absolute() {
        let html = html_from("notes/b", "See [[notes/a]] and [traits](traits.md).\n");

        assert!(html.contains(r#"href="/pages/notes/a""#), "got {html}");
        assert!(
            html.contains(r#"href="/pages/notes/traits""#),
            "a relative markdown link was not resolved against the source page: {html}"
        );
        assert!(
            !html.contains(r#"href="notes/a""#),
            "a relative wikilink href survived: {html}"
        );
    }

    /// Rendering a draft that has no slug yet must still work. Its relative
    /// links resolve as though it sat at the wiki root.
    #[test]
    fn a_source_less_render_resolves_against_the_wiki_root() {
        let html = html("See [[notes/a]] and [b](notes/b.md).\n");

        assert!(html.contains(r#"href="/pages/notes/a""#), "got {html}");
        assert!(html.contains(r#"href="/pages/notes/b""#), "got {html}");
    }

    #[test]
    fn external_links_are_left_alone() {
        let html = html("[site](https://example.com/a/b) and [mail](mailto:a@b.c)\n");

        assert!(
            html.contains(r#"href="https://example.com/a/b""#),
            "got {html}"
        );
        assert!(html.contains(r#"href="mailto:a@b.c""#), "got {html}");
    }

    /// A `#` in a slug is legal but would truncate the URL into a fragment if
    /// it went into the path verbatim.
    #[test]
    fn page_urls_percent_encode_their_segments() {
        assert_eq!(page_url("notes/a b"), "/pages/notes/a%20b");
        assert_eq!(page_url("notes/c#d"), "/pages/notes/c%23d");
        assert_eq!(page_url("notes/100%"), "/pages/notes/100%25");
        // Separators stay separators.
        assert_eq!(page_url("a/b/c"), "/pages/a/b/c");
        // Non-ASCII is encoded per UTF-8 byte.
        assert_eq!(page_url("caf\u{e9}"), "/pages/caf%C3%A9");
    }

    /// A wikilink whose target is not a valid slug is not a page link, so it
    /// must not be dressed up as one.
    #[test]
    fn an_invalid_wikilink_target_is_not_rewritten() {
        let html = html("See [[../../etc/passwd]].\n");

        assert!(
            !html.contains("/pages/"),
            "an invalid target was rewritten into a page URL: {html}"
        );
    }

    /// Links inside code are not links, in rendered output either.
    #[test]
    fn wikilinks_inside_code_are_not_rewritten() {
        let html = html("Write `[[notes/a]]` to link.\n");

        assert!(!html.contains("/pages/"), "got {html}");
    }

    // ------------------------------------------------------------- links

    fn links_from(source: &str, markdown: &str) -> Vec<Link> {
        extract_links(&Slug::parse(source).expect("valid slug"), markdown)
    }

    fn targets(source: &str, markdown: &str) -> Vec<String> {
        links_from(source, markdown)
            .into_iter()
            .map(|link| link.target)
            .collect()
    }

    #[test]
    fn extracts_wikilinks() {
        let links = links_from("index", "See [[notes/rust/async]] for more.\n");

        assert_eq!(links.len(), 1);
        assert_eq!(links[0].target, "notes/rust/async");
        assert_eq!(links[0].kind, LinkKind::Wiki);
    }

    #[test]
    fn extracts_wikilinks_with_display_text() {
        let links = links_from("index", "See [[notes/rust/async|the async notes]].\n");

        assert_eq!(links[0].target, "notes/rust/async");
        assert_eq!(links[0].display.as_deref(), Some("the async notes"));
    }

    /// `display` should say what the link says, not restate the target.
    #[test]
    fn display_text_that_repeats_the_target_is_dropped() {
        assert_eq!(links_from("index", "See [[notes/a]].\n")[0].display, None);
        assert_eq!(
            links_from("index", "See [notes/a](notes/a.md).\n")[0].display,
            None
        );
        // But a caption that differs is kept.
        assert_eq!(
            links_from("index", "See [the notes](notes/a.md).\n")[0].display,
            Some("the notes".to_owned())
        );
    }

    /// The reason extraction goes through the parser instead of a regex.
    #[test]
    fn wikilinks_inside_code_are_not_links() {
        assert!(targets("index", "Write `[[notes/rust/async]]` to link.\n").is_empty());
        assert!(targets("index", "```markdown\n[[notes/rust/async]]\n```\n").is_empty());
        assert!(
            targets("index", "    [[notes/rust/async]]\n").is_empty(),
            "indented code block was treated as prose"
        );
    }

    #[test]
    fn extracts_relative_markdown_links() {
        // From `notes/rust/async`, `../pinning.md` is `notes/pinning`.
        assert_eq!(
            targets("notes/rust/async", "See [pinning](../pinning.md).\n"),
            ["notes/pinning"]
        );
        // A sibling.
        assert_eq!(
            targets("notes/rust/async", "See [traits](traits.md).\n"),
            ["notes/rust/traits"]
        );
        // Explicitly relative.
        assert_eq!(
            targets("notes/rust/async", "See [traits](./traits.md).\n"),
            ["notes/rust/traits"]
        );
        // Absolute from the wiki root.
        assert_eq!(
            targets("notes/rust/async", "See [home](/index.md).\n"),
            ["index"]
        );
        // The extension is optional.
        assert_eq!(
            targets("notes/rust/async", "See [traits](traits).\n"),
            ["notes/rust/traits"]
        );
    }

    #[test]
    fn classifies_external_links() {
        let links = links_from(
            "index",
            "[site](https://example.com) and [mail](mailto:a@b.c) and [proto](//cdn.example.com/x)\n",
        );

        assert_eq!(links.len(), 3);
        assert!(links.iter().all(|link| link.kind == LinkKind::External));
        assert_eq!(links[0].target, "https://example.com");
    }

    #[test]
    fn ignores_fragments_and_images() {
        assert!(targets("index", "[top](#heading)\n").is_empty());
        assert!(
            targets("index", "![a picture](/images/x.png)\n").is_empty(),
            "an image is not a link to a page"
        );
    }

    #[test]
    fn drops_fragments_and_queries_from_page_targets() {
        assert_eq!(
            targets("index", "[a](notes/async.md#futures)\n"),
            ["notes/async"]
        );
        assert_eq!(
            targets("index", "[a](notes/async.md?v=2)\n"),
            ["notes/async"]
        );
    }

    /// A link that climbs out of the wiki is not a page you could create, so it
    /// is dropped rather than recorded as a wanted page.
    #[test]
    fn links_that_escape_the_wiki_are_dropped() {
        assert!(targets("index", "[out](../../etc/passwd)\n").is_empty());
        assert!(targets("notes/a", "[out](../../../x.md)\n").is_empty());
        assert!(targets("index", "[nowhere](/)\n").is_empty());
    }

    #[test]
    fn deduplicates_repeated_links() {
        let links = links_from(
            "index",
            "[[notes/a]] and again [[notes/a]] and [once more](notes/a.md).\n",
        );

        // Same target twice as a wikilink collapses; the markdown link to the
        // same page is a different kind and stays distinguishable.
        let wiki: Vec<_> = links
            .iter()
            .filter(|link| link.kind == LinkKind::Wiki)
            .collect();
        assert_eq!(wiki.len(), 1);
    }

    #[test]
    fn extracts_nothing_from_a_page_with_no_links() {
        assert!(targets("index", "# Just a heading\n\nAnd a paragraph.\n").is_empty());
    }

    #[test]
    fn wikilinks_render_as_anchors() {
        let html = html("See [[notes/rust/async|the notes]].\n");
        assert!(
            html.contains(r#"href="/pages/notes/rust/async""#),
            "got {html}"
        );
        assert!(html.contains("the notes"));
    }

    #[test]
    fn link_kinds_round_trip_through_their_labels() {
        for kind in [LinkKind::Wiki, LinkKind::Internal, LinkKind::External] {
            assert_eq!(LinkKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(LinkKind::parse("nonsense"), None);
    }

    #[test]
    fn counts_the_words_in_ordinary_prose() {
        assert_eq!(count_words("Knowledge branches off chaotically.\n"), 4);
        assert_eq!(count_words(""), 0);
        assert_eq!(count_words("   \n\n  \n"), 0);
    }

    /// Two paragraphs are not one long sentence, and neither are two cells.
    #[test]
    fn blocks_do_not_run_into_each_other() {
        assert_eq!(count_words("# One\n\nTwo three\n\n- four\n- five\n"), 5);
        assert_eq!(count_words("> quoted words here\n"), 3);
        assert_eq!(
            count_words("| a | b |\n|---|---|\n| c | d |\n"),
            4,
            "the rule row is punctuation, not two words"
        );
    }

    /// Emphasis splits a word in the AST, and the source is what says whether
    /// the pieces were one word or two.
    #[test]
    fn emphasis_does_not_change_the_count() {
        assert_eq!(count_words("un*believable*\n"), 1);
        assert_eq!(count_words("hello *world*\n"), 2);
        assert_eq!(count_words("**all** of it *emphasised*\n"), 4);
    }

    /// The reason this goes through the parser at all.
    #[test]
    fn code_is_not_prose() {
        assert_eq!(
            count_words(
                "Prose here.\n\n```rust\nfn main() { println!(\"lots of words\"); }\n```\n"
            ),
            2
        );
        assert_eq!(count_words("Call `std::mem::swap` now.\n"), 2);
        assert_eq!(
            count_words("~~~\nnot counted at all\n~~~\n"),
            0,
            "a tilde fence is a fence"
        );
    }

    /// A count that moved when somebody indented a code block would send the
    /// writer looking for words they never wrote.
    #[test]
    fn an_indented_code_block_is_code_too() {
        assert_eq!(
            count_words("Prose.\n\n    fn main() { one two three }\n"),
            1
        );
    }

    /// A link says something; where it points is not part of what it says.
    #[test]
    fn a_link_contributes_its_text_and_not_its_target() {
        assert_eq!(
            count_words("See [the async notes](notes/rust/async.md).\n"),
            4
        );
        assert_eq!(
            count_words("See [[notes/rust/async|the async notes]].\n"),
            4
        );
        assert_eq!(
            count_words("See [[notes/rust/async]].\n"),
            2,
            "a bare wikilink displays its slug, so the slug is the word"
        );
        assert_eq!(count_words("Read <https://example.com/a/b/c>.\n"), 2);
    }

    /// Alt text describes a picture rather than being read as part of the prose.
    #[test]
    fn image_alt_text_is_not_prose() {
        assert_eq!(count_words("Look: ![a red bicycle](bike.png)\n"), 1);
    }

    /// The markup is not words. What is *between* the markup still is, and the
    /// two halves of that are not the same rule.
    ///
    /// A raw HTML **block** takes its whole contents with it, because the
    /// renderer drops the block and nothing in it reaches the page. An inline
    /// tag does not: `<span>` is dropped and the words it wraps are still
    /// rendered, still read, and still words. Counting them would have been the
    /// easy thing to write and would have made the number disagree with the page.
    #[test]
    fn markup_is_not_words_but_the_words_inside_it_are() {
        assert_eq!(count_words("<div>markup words here</div>\n"), 0);
        assert_eq!(count_words("Prose <span>and markup</span> prose.\n"), 4);
    }

    /// A footnote's text is prose; its marker is not.
    #[test]
    fn footnote_text_counts_and_the_marker_does_not() {
        assert_eq!(
            count_words("A claim.[^1]\n\n[^1]: Two words.\n"),
            4,
            "two in the claim, two in the note"
        );
    }

    /// Punctuation on its own is not a word, and an apostrophe does not make two.
    #[test]
    fn a_word_needs_a_letter_or_a_digit_in_it() {
        assert_eq!(count_words("it's a 42 --- ... word\n"), 4);
    }
}
