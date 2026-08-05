//! Markdown: rendering page bodies, and pulling the link graph out of them.
//!
//! Rendering is opt-in: the API serves raw markdown by default, because that is
//! what an agent can reason about and edit. HTML is for the browser.
//!
//! Links are extracted from comrak's AST rather than by scanning the source.
//! That is what makes `[[not a link]]` inside a code fence stay inside the code
//! fence — the parser has already decided what is code and what is prose, and a
//! regex over the raw text would have to relitigate it and get it wrong.

use comrak::nodes::{AstNode, NodeValue};
use comrak::{Arena, Options};

use crate::slug::Slug;

/// Render a markdown body to HTML.
///
/// Raw HTML in the source is **escaped, not passed through**. Rhizowiki is
/// single-user and loopback-bound, so this is not guarding against a hostile
/// author — but page content arrives over an API that agents write to, and
/// rendering `<script>` from an indirect source into the dashboard is the kind
/// of thing that is very hard to notice and very easy to avoid. Turning this on
/// should be a deliberate, separate decision.
pub fn render(markdown: &str) -> String {
    comrak::markdown_to_html(markdown, &options())
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
            NodeValue::Link(markdown_link) => classify(source, &markdown_link.url)
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
/// Returns `None` for things that are not page links at all: bare fragments,
/// and internal-looking targets that cannot be normalised into a valid slug.
/// Those are dropped rather than recorded as broken, because a "wanted page"
/// should be something you could go and create — `../../outside-the-wiki` is
/// not.
fn classify(source: &Slug, url: &str) -> Option<(String, LinkKind)> {
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
        let base = source.parent().unwrap_or("");
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

    #[test]
    fn renders_ordinary_markdown() {
        let html = render("# Heading\n\nSome *emphasis* and a [link](https://example.com).\n");

        assert!(html.contains("<h1>Heading</h1>"));
        assert!(html.contains("<em>emphasis</em>"));
        assert!(html.contains(r#"<a href="https://example.com">link</a>"#));
    }

    #[test]
    fn renders_gfm_extensions() {
        assert!(render("~~gone~~").contains("<del>gone</del>"));
        assert!(render("| a | b |\n|---|---|\n| 1 | 2 |\n").contains("<table>"));
        assert!(render("- [x] done\n").contains("checked"));
        assert!(render("Visit https://example.com today").contains("<a href="));
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
        let html = render("<script>alert('xss')</script>\n");

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
        let html = render("Text with <img src=x onerror=alert(1)> inline.\n");

        assert!(
            !html.contains("<img"),
            "inline HTML was passed through: {html}"
        );
    }

    /// A javascript: URL must not survive as a clickable link.
    #[test]
    fn dangerous_link_schemes_are_neutralised() {
        let html = render("[click me](javascript:alert(1))\n");

        assert!(
            !html.contains("href=\"javascript:"),
            "javascript: URL survived: {html}"
        );
    }

    #[test]
    fn an_empty_body_renders_to_nothing() {
        assert_eq!(render("").trim(), "");
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
        let html = render("See [[notes/rust/async|the notes]].\n");
        assert!(html.contains(r#"href="notes/rust/async""#), "got {html}");
        assert!(html.contains("the notes"));
    }

    #[test]
    fn link_kinds_round_trip_through_their_labels() {
        for kind in [LinkKind::Wiki, LinkKind::Internal, LinkKind::External] {
            assert_eq!(LinkKind::parse(kind.as_str()), Some(kind));
        }
        assert_eq!(LinkKind::parse("nonsense"), None);
    }
}
