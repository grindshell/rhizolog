//! Assembling a tree of pages into one document.
//!
//! A wiki page is something you decided. A manuscript is something you hand
//! over, and there is no way to hand over nine pages. Compiling walks a page's
//! `contents:` list, and its chapters' lists after that, and returns one
//! document plus a **manifest** saying where every part of it came from.
//!
//! Two rules shape everything here, and both are argued in
//! `knowledge-base/long-form.md`:
//!
//! - **Structure is frontmatter.** A link written in prose is never a section,
//!   on any page. `contents:` is the only thing that assembles anything, which
//!   is the same answer [`crate::times`] gives for what counts as time spent on
//!   a page and for the same reason: these edges decide the numbers, so they
//!   have to be a deliberate act rather than an accident of writing.
//! - **A body is emitted as written.** Only headings are touched, and only their
//!   level. The manifest's offsets are into the bytes that come back, so a
//!   caller can map any position in a compiled book to the page that owns it.
//!
//! The manifest is the reason this is not a blob. Without it the output is text
//! nothing can point into, and the caller most likely to be handed one is an
//! assistant that will then talk about "the ferry scene" with no way to say
//! where it is.

use std::collections::HashSet;

use chrono::{DateTime, Utc};
use comrak::nodes::NodeValue;
use comrak::{Arena, Options};

use crate::markdown;
use crate::page::Page;
use crate::slug::Slug;

/// How deep a chain of contents pages may go.
///
/// A book is title, part, chapter, scene. Sixteen is far past any real
/// structure and near enough to catch a mistake. Cycle detection already ends a
/// loop, so this catches the thing that is not a loop: a chain of sixty contents
/// pages each holding the next terminates perfectly well and is not a book.
pub const MAX_DEPTH: usize = 16;

/// How many sections one document may hold. A chapter each, in a work nobody
/// has written.
pub const MAX_SECTIONS: usize = 2_000;

/// How large the output may get. Roughly a million words, which is several
/// books.
pub const MAX_BYTES: usize = 8 * 1024 * 1024;

/// Which ceiling a compile ran into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Limit {
    Depth,
    Sections,
    Bytes,
}

impl Limit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Depth => "depth",
            Self::Sections => "sections",
            Self::Bytes => "bytes",
        }
    }

    pub fn ceiling(self) -> usize {
        match self {
            Self::Depth => MAX_DEPTH,
            Self::Sections => MAX_SECTIONS,
            Self::Bytes => MAX_BYTES,
        }
    }
}

/// What happened to one entry in a contents list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Its body is in the document.
    Included,
    /// Nothing is written at that slug, **or** the caller may not read it.
    ///
    /// Deliberately one answer rather than two. Telling them apart would confirm
    /// that a page exists at a slug somebody guessed, which is the same oracle
    /// `404, never 403` exists to withhold everywhere else.
    Wanted,
    /// Not a slug at all: a URL, a `..` path, an empty string.
    Invalid,
    /// Already emitted earlier in this compile.
    ///
    /// Not called `cycle`, because the commoner case is not one: an appendix
    /// listed under two parts is a diamond, and a name that said cycle would
    /// send somebody looking for a loop that is not there.
    Duplicate,
    /// The page exists and its frontmatter will not parse.
    ///
    /// Reported as itself, unlike [`Status::Wanted`], because a malformed page
    /// is already reported to anybody who asks for it: saying so here discloses
    /// nothing new, and hiding it would swallow a real fault.
    Unreadable,
    /// The page says `compile: false`, or something above it does.
    ///
    /// A sixth status rather than an absence, for the reason all five others
    /// are one: a section keeps its position whatever happened to it. Removing
    /// the entry from `contents:` would also say "not in the book", and would
    /// throw away **where it went**, which is the one thing the contents list
    /// knows and a wikilink does not.
    ///
    /// **Excluding a contents page excludes everything under it.** Scrivener
    /// takes the other option, where a folder's children compile according to
    /// their own setting, and it is wrong here because of headings: a part
    /// contributes its heading and its epigraph, so dropping only its body would
    /// leave its chapters in the document with the part's heading gone, silently
    /// promoting them under the previous part.
    Excluded,
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Included => "included",
            Self::Wanted => "wanted",
            Self::Invalid => "invalid",
            Self::Duplicate => "duplicate",
            Self::Unreadable => "unreadable",
            Self::Excluded => "excluded",
        }
    }
}

/// One entry in the manifest.
///
/// Every entry keeps its position whatever its status. A manuscript short of a
/// chapter says where the chapter was going to be, which is the whole difference
/// between a gap and an omission.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Section {
    /// The slug as the contents list wrote it, so an `invalid` entry can be
    /// found and fixed.
    pub slug: String,
    pub title: Option<String>,
    /// What the page says it is for. Absent when it says nothing, and absent
    /// for anything that is not `included`, exactly as `title` is.
    pub synopsis: Option<String>,
    /// What stage of drafting the page is at, as written. Absent on the same
    /// terms as `synopsis`.
    pub stage: Option<String>,
    /// The page's own `target`, from its frontmatter.
    ///
    /// Measured against [`Section::subtree`] rather than [`Section::words`],
    /// which is the whole reason the second number exists: one rule, recursive,
    /// no second concept. On a leaf the two are equal; on a part page `words` is
    /// the epigraph and `target` means the part.
    pub target: Option<u64>,
    /// The page whose `contents:` list named this entry.
    ///
    /// Absent on the root and on a `?style=` preamble, which nothing named:
    /// those are pages the request asked for rather than pages the spine
    /// reaches. Always absent or present together with [`Section::ordinal`].
    pub parent: Option<String>,
    /// Where in that list, counting from zero.
    ///
    /// The **identity** of the entry, and not a position in this manifest. A
    /// contents list may name the same child twice, which is what `page_parts`
    /// is keyed `(src_slug, ordinal)` for, and a parent below something excluded
    /// and something included is walked down both paths, so its children appear
    /// here twice with the same ordinals. Anything reconstructing a contents
    /// list has to key on this rather than counting rows.
    pub ordinal: Option<usize>,
    /// How many contents lists deep this page sits. The root is zero.
    pub depth: usize,
    /// This section's own body, in words. Zero for everything but `included`.
    pub words: u64,
    /// This section's words plus everything emitted beneath it.
    ///
    /// A `duplicate` or `excluded` section contributes nothing to any ancestor's
    /// subtree, because it contributed nothing to the document, so this always
    /// describes what a reader would actually get.
    pub subtree: u64,
    /// Where this section's bytes begin in the output.
    pub offset: usize,
    /// How many bytes they run for. Zero for everything but `included`.
    pub length: usize,
    pub status: Status,
}

/// A compiled document and the map back to the pages it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub markdown: String,
    pub sections: Vec<Section>,
    /// Words across every included section.
    pub words: u64,
    /// The root's `target`, if it names one.
    pub target: Option<u64>,
    /// The root's `due`, if what its frontmatter says is a day.
    ///
    /// Here beside `target` for the same reason that one is: it is the root's
    /// own frontmatter travelling with the document, so a caller holding a
    /// compile does not have to read the page again to know what the work is
    /// aiming at and when. Nothing in the assembly consults either.
    pub due: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CompileError {
    /// Nothing at the root slug, or nothing this caller may read.
    RootNotFound { slug: Slug },
    /// A ceiling was reached. Named, with the slug it was reached at.
    TooLarge { limit: Limit, at: String },
}

/// What one fetch produced.
///
/// The audience check belongs to whoever implements [`Pages`], and it collapses
/// into [`Fetched::Missing`] on purpose: see [`Status::Wanted`].
#[derive(Debug, Clone)]
pub enum Fetched {
    Page(Box<Page>),
    Missing,
    Unreadable,
}

/// Where a compile gets its pages.
///
/// A trait rather than a `&Store` so the walk can be tested over a map, without
/// a filesystem and without an audience. Every interesting case here is a shape
/// of tree rather than a shape of disk.
pub trait Pages {
    fn fetch(&self, slug: &Slug) -> impl Future<Output = Fetched> + Send;
}

/// Assemble `root` and everything its contents lists reach.
///
/// `style` is prepended verbatim, at depth zero, and appears in the manifest
/// like anything else that is in the bytes. It is a real part of the document,
/// so leaving it out of the map would make every offset after it a lie.
pub async fn compile(
    root: &Slug,
    style: Option<&Slug>,
    pages: &impl Pages,
) -> Result<Compiled, CompileError> {
    let mut out = String::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut words = 0;

    // Fetched before anything else, because a root that is not there is an
    // error rather than a gap: a manuscript with no first page is not a short
    // manuscript.
    let Fetched::Page(root_page) = pages.fetch(root).await else {
        return Err(CompileError::RootNotFound { slug: root.clone() });
    };
    let target = root_page.frontmatter.target;
    let due = root_page.due();

    // A preamble is emitted whatever it says about `compile`. That field means
    // "not part of the book", and a style page is not part of the book: it is a
    // page this caller named in this request, and dropping it silently would
    // leave them without the preamble they asked for and no reason given.
    if let Some(style) = style
        && let Fetched::Page(page) = pages.fetch(style).await
    {
        emit(&mut out, &mut sections, &mut words, &page, 0, None)?;
    }

    // An explicit stack rather than recursion, which for an async walk would
    // mean boxing every level. Children are pushed in reverse so they come off
    // in the order the contents list names them.
    let mut stack: Vec<Pending> = vec![Pending {
        raw: root.to_string(),
        depth: 0,
        inherited: false,
        origin: None,
    }];
    let mut emitted: HashSet<String> = HashSet::new();
    // Pages already walked while excluded. Nothing else can end an excluded
    // cycle: `emitted` is deliberately not consulted or written on that path, so
    // without this a two-page loop below a `compile: false` part would descend
    // until it hit the depth limit and turn a harmless mistake into a refusal.
    let mut skipped: HashSet<String> = HashSet::new();

    while let Some(Pending {
        raw,
        depth,
        inherited,
        origin,
    }) = stack.pop()
    {
        if sections.len() >= MAX_SECTIONS {
            return Err(CompileError::TooLarge {
                limit: Limit::Sections,
                at: raw,
            });
        }
        if depth > MAX_DEPTH {
            return Err(CompileError::TooLarge {
                limit: Limit::Depth,
                at: raw,
            });
        }

        let Ok(slug) = Slug::parse(&raw) else {
            sections.push(gap(raw, depth, out.len(), Status::Invalid, origin));
            continue;
        };

        // Only on the included path. An excluded page is never emitted, so it
        // can never be the thing a later entry is a duplicate *of*, which is
        // what makes an appendix listed under an excluded part and an included
        // one come out `included` once and `duplicate` nowhere. No special case:
        // the rule is that `duplicate` means already emitted.
        if !inherited && emitted.contains(slug.as_str()) {
            sections.push(gap(raw, depth, out.len(), Status::Duplicate, origin));
            continue;
        }

        match pages.fetch(&slug).await {
            Fetched::Missing => sections.push(gap(raw, depth, out.len(), Status::Wanted, origin)),
            Fetched::Unreadable => {
                sections.push(gap(raw, depth, out.len(), Status::Unreadable, origin))
            }
            Fetched::Page(page) => {
                let excluded = inherited || !page.compiled();

                if excluded {
                    sections.push(gap(raw, depth, out.len(), Status::Excluded, origin));

                    // Still walked, so every chapter under a cut part keeps its
                    // position in the manifest rather than vanishing with it.
                    // Seen twice, it stops: see `skipped` above.
                    if skipped.insert(slug.to_string()) {
                        descend(&mut stack, &page, depth, true);
                    }
                    continue;
                }

                emitted.insert(slug.to_string());
                emit(&mut out, &mut sections, &mut words, &page, depth, origin)?;
                descend(&mut stack, &page, depth, false);
            }
        }
    }

    accumulate_subtrees(&mut sections);

    Ok(Compiled {
        markdown: out,
        sections,
        words,
        target,
        due,
    })
}

/// One entry still to be walked.
///
/// The flag is whether this entry is inside a subtree something excluded. It is
/// carried down rather than looked up, because "excluded" is a fact about a path
/// through the tree and not about a page: the same appendix can be excluded under
/// one part and included under another.
struct Pending {
    raw: String,
    depth: usize,
    inherited: bool,
    /// Which contents list named it, and where in that list. `None` for the root
    /// and for a `?style=` preamble, which nothing named.
    origin: Option<(String, usize)>,
}

/// Queue everything `page` assembles, in the order its contents list names them.
///
/// Pushed in reverse so they come off the stack forwards, which is why the
/// ordinal is taken before the reversal rather than after it: it is the index in
/// the **contents list**, not the order anything is visited in.
fn descend(stack: &mut Vec<Pending>, page: &Page, depth: usize, inherited: bool) {
    let slug = page.slug.to_string();

    for (ordinal, child) in page.contents().unwrap_or_default().iter().enumerate().rev() {
        stack.push(Pending {
            raw: child.clone(),
            depth: depth + 1,
            inherited,
            origin: Some((slug.clone(), ordinal)),
        });
    }
}

/// Fill in each section's `subtree` from the sections beneath it.
///
/// The walk is depth-first and pre-order, so a section's descendants are exactly
/// the run of entries after it whose depth is greater than its own, and the run
/// ends at the first entry that is not. That makes this one pass over a list
/// rather than a second traversal, and it is why the walk itself does not have
/// to carry a parent along.
///
/// A `style` page sits at depth zero beside the root and is not its ancestor,
/// which this gets right for free: the root is the first entry after it at depth
/// zero, so the run is empty and the preamble's subtree is its own words.
fn accumulate_subtrees(sections: &mut [Section]) {
    for index in 0..sections.len() {
        let depth = sections[index].depth;
        let mut total = sections[index].words;

        for below in &sections[index + 1..] {
            if below.depth <= depth {
                break;
            }
            // Anything not `included` carries zero words, so a gap, a repeat and
            // a cut chapter all add nothing rather than each needing a case.
            total += below.words;
        }

        sections[index].subtree = total;
    }
}

/// Append one page's body and record where it landed.
fn emit(
    out: &mut String,
    sections: &mut Vec<Section>,
    words: &mut u64,
    page: &Page,
    depth: usize,
    origin: Option<(String, usize)>,
) -> Result<(), CompileError> {
    separate(out);

    let body = shift_headings(&page.body, depth);
    let offset = out.len();
    out.push_str(&body);

    if out.len() > MAX_BYTES {
        return Err(CompileError::TooLarge {
            limit: Limit::Bytes,
            at: page.slug.to_string(),
        });
    }

    let counted = page.words();
    *words += counted;

    let (parent, ordinal) = split(origin);

    sections.push(Section {
        slug: page.slug.to_string(),
        title: Some(page.title()),
        synopsis: page.synopsis().map(str::to_owned),
        stage: page.stage().map(str::to_owned),
        target: page.frontmatter.target,
        parent,
        ordinal,
        depth,
        words: counted,
        // Filled in by `accumulate_subtrees` once the whole tree is known, since
        // this is a number about what comes after.
        subtree: 0,
        offset,
        length: body.len(),
        status: Status::Included,
    });

    Ok(())
}

/// A section that is in the manifest and not in the document.
///
/// Everything a page would have said about itself is absent, which is the rule
/// `title` already followed: what a caller is being told is that nothing was
/// emitted here, and a card for a chapter that is not in the book would be
/// describing something the reader will not get.
fn gap(
    slug: String,
    depth: usize,
    offset: usize,
    status: Status,
    origin: Option<(String, usize)>,
) -> Section {
    let (parent, ordinal) = split(origin);

    Section {
        slug,
        title: None,
        synopsis: None,
        stage: None,
        target: None,
        // Kept where everything else about the page is dropped, and deliberately.
        // The other fields describe a page a reader will not get; these two
        // describe the **entry**, which is exactly what is still there and
        // exactly what somebody fixing a typo or reordering a spine needs. A gap
        // that could not say which list named it would be a gap nothing could
        // move.
        parent,
        ordinal,
        depth,
        words: 0,
        subtree: 0,
        offset,
        length: 0,
        status,
    }
}

/// A parent and an ordinal are absent together or present together.
fn split(origin: Option<(String, usize)>) -> (Option<String>, Option<usize>) {
    match origin {
        Some((parent, ordinal)) => (Some(parent), Some(ordinal)),
        None => (None, None),
    }
}

/// Put a blank line between two sections.
///
/// Only ever *adds* separators, never trims what a body ends with, so a
/// section's own bytes are exactly its own. Without it the last line of one
/// chapter and the first line of the next become one paragraph, which is the
/// kind of error that reads as an editing mistake rather than a tooling one.
fn separate(out: &mut String) {
    if out.is_empty() {
        return;
    }
    while !out.ends_with("\n\n") {
        out.push('\n');
    }
}

/// Push every heading in a body down by `by` levels.
///
/// An H1 in a chapter inserted at depth one becomes an H2 under the book's H1,
/// so the compiled document has one hierarchy instead of a dozen competing ones.
///
/// The headings are found through comrak rather than by scanning for `#`, for
/// the reason the whole of [`crate::markdown`] gives: the parser has already
/// decided what is a heading and what is a `#` inside a code fence. Only the
/// heading lines are rewritten; every other byte is passed through, which is
/// what lets the manifest promise that a section's bytes are its own.
///
/// Two things it has to do that are not obvious:
///
/// - **Levels clamp at six.** Markdown has no `#######`, so a heading that would
///   be pushed past six stops there rather than becoming a paragraph beginning
///   with hashes. Deep books lose some hierarchy at the bottom; the alternative
///   loses the heading entirely.
/// - **A setext heading becomes an ATX one.** `Title` over `=====` has no level
///   marker to change, so there is nothing to shift. Its source lines are kept
///   verbatim after the marker, so inline formatting survives even though the
///   spelling changes. This happens only in the compiled output; the file is
///   untouched.
pub fn shift_headings(body: &str, by: usize) -> String {
    if by == 0 || body.is_empty() {
        return body.to_owned();
    }

    let arena = Arena::new();
    let root = comrak::parse_document(&arena, body, &options());

    // (start line, end line, new level, setext), all 1-based from comrak.
    let mut headings: Vec<(usize, usize, usize, bool)> = Vec::new();
    for node in root.descendants() {
        if let NodeValue::Heading(heading) = node.data.borrow().value {
            let position = node.data.borrow().sourcepos;
            let level = (heading.level as usize + by).min(6);
            headings.push((
                position.start.line,
                position.end.line,
                level,
                heading.setext,
            ));
        }
    }

    if headings.is_empty() {
        return body.to_owned();
    }

    let lines: Vec<&str> = body.split_inclusive('\n').collect();
    let mut out = String::with_capacity(body.len() + headings.len() * by);
    let mut line_number = 0;

    while line_number < lines.len() {
        let current = line_number + 1;
        let Some(&(start, end, level, setext)) =
            headings.iter().find(|(start, ..)| *start == current)
        else {
            out.push_str(lines[line_number]);
            line_number += 1;
            continue;
        };

        let marker = "#".repeat(level);

        if setext {
            // The text is every line but the underline, kept as written so that
            // emphasis and links inside a heading survive the change of
            // spelling.
            let text: Vec<&str> = lines[start - 1..end.saturating_sub(1).max(start - 1)]
                .iter()
                .map(|line| line.trim_end_matches(['\r', '\n']).trim())
                .collect();
            out.push_str(&marker);
            out.push(' ');
            out.push_str(&text.join(" "));
            out.push_str(line_ending(lines[end - 1]));
            line_number = end;
        } else {
            let line = lines[line_number];
            let indent: String = line.chars().take_while(|c| c.is_whitespace()).collect();
            let rest = line[indent.len()..].trim_start_matches('#');
            out.push_str(&indent);
            out.push_str(&marker);
            out.push_str(rest);
            line_number += 1;
        }
    }

    out
}

/// Whatever ended this line, so a rewritten heading does not change the file's
/// line endings. Windows editors write CRLF and this all runs on Windows.
fn line_ending(line: &str) -> &str {
    if line.ends_with("\r\n") {
        "\r\n"
    } else if line.ends_with('\n') {
        "\n"
    } else {
        ""
    }
}

/// The same options the renderer and the link extractor use, so all three agree
/// about what a heading, a fence and a wikilink are.
fn options() -> Options<'static> {
    markdown::parser_options()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, Utc};
    use std::collections::HashMap;

    /// A wiki in a map. Every case worth testing here is a shape of tree.
    struct Wiki {
        pages: HashMap<String, Fetched>,
    }

    impl Wiki {
        fn new() -> Self {
            Self {
                pages: HashMap::new(),
            }
        }

        fn page(mut self, slug: &str, text: &str) -> Self {
            let page = Page::from_markdown(
                Slug::parse(slug).expect("valid slug"),
                text,
                DateTime::parse_from_rfc3339("2026-08-05T12:00:00Z")
                    .unwrap()
                    .with_timezone(&Utc),
            )
            .expect("page parses");
            self.pages
                .insert(slug.to_owned(), Fetched::Page(Box::new(page)));
            self
        }

        fn unreadable(mut self, slug: &str) -> Self {
            self.pages.insert(slug.to_owned(), Fetched::Unreadable);
            self
        }
    }

    impl Pages for Wiki {
        async fn fetch(&self, slug: &Slug) -> Fetched {
            match self.pages.get(slug.as_str()) {
                Some(Fetched::Page(page)) => Fetched::Page(page.clone()),
                Some(Fetched::Unreadable) => Fetched::Unreadable,
                _ => Fetched::Missing,
            }
        }
    }

    async fn compiled(wiki: &Wiki, root: &str) -> Compiled {
        compile(&Slug::parse(root).unwrap(), None, wiki)
            .await
            .expect("compiles")
    }

    fn statuses(compiled: &Compiled) -> Vec<(&str, Status)> {
        compiled
            .sections
            .iter()
            .map(|section| (section.slug.as_str(), section.status))
            .collect()
    }

    /// A parent's `contents:` list, rebuilt out of the manifest alone.
    ///
    /// This is the whole reason `parent` and `ordinal` are reported, so it is
    /// written here the way a client has to write it: keyed on the ordinal, never
    /// on the order rows appear or on how many of them there are.
    fn rebuilt(compiled: &Compiled, parent: &str) -> Vec<String> {
        let mut entries: Vec<(usize, &str)> = compiled
            .sections
            .iter()
            .filter(|section| section.parent.as_deref() == Some(parent))
            .filter_map(|section| section.ordinal.map(|at| (at, section.slug.as_str())))
            .collect();

        entries.sort_by_key(|(at, _)| *at);
        entries.dedup_by_key(|(at, _)| *at);
        entries
            .into_iter()
            .map(|(_, slug)| slug.to_owned())
            .collect()
    }

    #[tokio::test]
    async fn a_leaf_page_compiles_to_itself() {
        let wiki = Wiki::new().page("solo", "# Solo\n\nJust the one.\n");
        let result = compiled(&wiki, "solo").await;

        assert_eq!(result.markdown, "# Solo\n\nJust the one.\n");
        assert_eq!(result.sections.len(), 1);
        assert_eq!(result.sections[0].depth, 0);
        assert_eq!(result.words, 4);
    }

    /// The body comes first and the contents after it, which is what puts a part
    /// title and an epigraph where they belong with no positional convention at
    /// all.
    #[tokio::test]
    async fn a_body_comes_before_the_pages_it_assembles() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/two]\n---\n\n# Book\n\nAn epigraph.\n",
            )
            .page("book/one", "# One\n\nFirst.\n")
            .page("book/two", "# Two\n\nSecond.\n");

        let result = compiled(&wiki, "book").await;

        assert_eq!(
            result.markdown,
            "\n# Book\n\nAn epigraph.\n\n## One\n\nFirst.\n\n## Two\n\nSecond.\n"
        );
        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("book/one", Status::Included),
                ("book/two", Status::Included),
            ]
        );
    }

    /// Depth is structural, so a part's chapters land two levels down.
    #[tokio::test]
    async fn nesting_shifts_headings_by_depth() {
        let wiki = Wiki::new()
            .page("book", "---\ncontents: [book/one]\n---\n\n# Book\n")
            .page(
                "book/one",
                "---\ncontents: [book/one/opening]\n---\n\n# Part One\n",
            )
            .page("book/one/opening", "# Opening\n\nProse.\n");

        let result = compiled(&wiki, "book").await;

        assert!(result.markdown.contains("# Book"));
        assert!(result.markdown.contains("## Part One"));
        assert!(result.markdown.contains("### Opening"));
        assert_eq!(result.sections[2].depth, 2);
    }

    /// The promise the manifest makes: every included section's bytes are at the
    /// offset it claims, exactly once.
    #[tokio::test]
    async fn every_offset_indexes_into_the_output() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/two]\n---\n\n# Book\n",
            )
            .page("book/one", "# One\n\nFirst chapter.\n")
            .page("book/two", "# Two\n\nSecond chapter.\n");

        let result = compiled(&wiki, "book").await;

        for section in &result.sections {
            if section.status != Status::Included {
                continue;
            }
            let slice = &result.markdown[section.offset..section.offset + section.length];
            assert!(
                slice.contains(section.title.as_deref().unwrap()),
                "{} did not sit at its own offset: {slice:?}",
                section.slug
            );
        }
    }

    #[tokio::test]
    async fn compiling_twice_is_byte_identical() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/two]\n---\n\n# Book\n",
            )
            .page("book/one", "# One\n\nFirst.\n")
            .page("book/two", "# Two\n\nSecond.\n");

        assert_eq!(
            compiled(&wiki, "book").await.markdown,
            compiled(&wiki, "book").await.markdown
        );
    }

    /// A chapter nobody has written yet holds its place, so the manuscript says
    /// where it was going rather than quietly closing the gap.
    #[tokio::test]
    async fn a_wanted_chapter_keeps_its_position() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/missing, book/two]\n---\n\n# Book\n",
            )
            .page("book/one", "# One\n")
            .page("book/two", "# Two\n");

        let result = compiled(&wiki, "book").await;

        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("book/one", Status::Included),
                ("book/missing", Status::Wanted),
                ("book/two", Status::Included),
            ]
        );
    }

    /// Both shapes of repeat, and the reason the status is not called `cycle`.
    #[tokio::test]
    async fn a_repeat_is_reported_rather_than_dropped() {
        let loop_wiki = Wiki::new()
            .page("a", "---\ncontents: [b]\n---\n\n# A\n")
            .page("b", "---\ncontents: [a]\n---\n\n# B\n");
        assert_eq!(
            statuses(&compiled(&loop_wiki, "a").await),
            [
                ("a", Status::Included),
                ("b", Status::Included),
                ("a", Status::Duplicate),
            ]
        );

        // Not a cycle at all: one appendix under two parts.
        let diamond = Wiki::new()
            .page("book", "---\ncontents: [one, two]\n---\n\n# Book\n")
            .page("one", "---\ncontents: [appendix]\n---\n\n# One\n")
            .page("two", "---\ncontents: [appendix]\n---\n\n# Two\n")
            .page("appendix", "# Appendix\n");

        let result = compiled(&diamond, "book").await;
        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("one", Status::Included),
                ("appendix", Status::Included),
                ("two", Status::Included),
                ("appendix", Status::Duplicate),
            ]
        );
    }

    /// A page listed twice under one parent keeps both positions. This is what
    /// `links` could not represent and why the spine is its own table.
    #[tokio::test]
    async fn the_same_child_twice_under_one_parent_keeps_both_positions() {
        let wiki = Wiki::new()
            .page("book", "---\ncontents: [one, one]\n---\n\n# Book\n")
            .page("one", "# One\n");

        assert_eq!(
            statuses(&compiled(&wiki, "book").await),
            [
                ("book", Status::Included),
                ("one", Status::Included),
                ("one", Status::Duplicate),
            ]
        );
    }

    /// Every relative spelling, from a page that is nested rather than at the
    /// root. None of them may reach a page beside the one holding the list.
    #[tokio::test]
    async fn a_relative_entry_is_refused_rather_than_resolved() {
        let wiki = Wiki::new()
            .page(
                "book/one",
                "---\ncontents: [\"./opening\", \"../two/opening\", opening]\n---\n\n# One\n",
            )
            .page("book/one/opening", "# Opening\n")
            .page("opening", "# A top-level page\n");

        let result = compiled(&wiki, "book/one").await;

        assert_eq!(
            statuses(&result),
            [
                ("book/one", Status::Included),
                ("./opening", Status::Invalid),
                ("../two/opening", Status::Invalid),
                // A bare basename is a perfectly good slug, so it resolves from
                // the root and finds the page that is actually there. It never
                // reaches `book/one/opening`.
                ("opening", Status::Included),
            ]
        );
        assert!(result.markdown.contains("A top-level page"));
        assert!(!result.markdown.contains("# Opening"));
    }

    #[tokio::test]
    async fn a_url_in_a_contents_list_is_invalid_and_fetches_nothing() {
        let wiki = Wiki::new().page(
            "book",
            "---\ncontents: [\"https://example.com/a\", \"\"]\n---\n\n# Book\n",
        );

        assert_eq!(
            statuses(&compiled(&wiki, "book").await),
            [
                ("book", Status::Included),
                ("https://example.com/a", Status::Invalid),
                ("", Status::Invalid),
            ]
        );
    }

    /// A page that will not parse is reported as itself; one that is merely not
    /// there, or not readable, is `wanted`.
    #[tokio::test]
    async fn a_malformed_chapter_says_so() {
        let wiki = Wiki::new()
            .page("book", "---\ncontents: [broken]\n---\n\n# Book\n")
            .unreadable("broken");

        assert_eq!(
            statuses(&compiled(&wiki, "book").await),
            [("book", Status::Included), ("broken", Status::Unreadable)]
        );
    }

    #[tokio::test]
    async fn a_root_that_is_not_there_is_an_error_rather_than_a_gap() {
        let wiki = Wiki::new();
        let error = compile(&Slug::parse("book").unwrap(), None, &wiki)
            .await
            .expect_err("a missing root is an error");

        assert!(matches!(error, CompileError::RootNotFound { .. }));
    }

    #[tokio::test]
    async fn a_chain_deeper_than_the_limit_is_refused() {
        let mut wiki = Wiki::new();
        for level in 0..=MAX_DEPTH + 1 {
            wiki = wiki.page(
                &format!("p{level}"),
                &format!("---\ncontents: [p{}]\n---\n\n# Level\n", level + 1),
            );
        }

        let error = compile(&Slug::parse("p0").unwrap(), None, &wiki)
            .await
            .expect_err("too deep");

        assert!(matches!(
            error,
            CompileError::TooLarge {
                limit: Limit::Depth,
                ..
            }
        ));
    }

    /// The style page is prepended and is in the manifest, because it is in the
    /// bytes. Leaving it out would make every offset after it wrong.
    #[tokio::test]
    async fn a_style_page_is_prepended_and_mapped() {
        let wiki = Wiki::new()
            .page("book", "# Book\n\nProse.\n")
            .page("rules/voice", "# Voice\n\nPast tense.\n");

        let result = compile(
            &Slug::parse("book").unwrap(),
            Some(&Slug::parse("rules/voice").unwrap()),
            &wiki,
        )
        .await
        .expect("compiles");

        assert!(result.markdown.starts_with("# Voice"));
        assert_eq!(result.sections[0].slug, "rules/voice");
        assert_eq!(result.sections[1].slug, "book");
        let book = &result.sections[1];
        assert_eq!(
            &result.markdown[book.offset..book.offset + book.length],
            "# Book\n\nProse.\n"
        );
    }

    // ----------------------------------------------------------- drafting

    fn find<'a>(compiled: &'a Compiled, slug: &str) -> &'a Section {
        compiled
            .sections
            .iter()
            .find(|section| section.slug == slug)
            .unwrap_or_else(|| panic!("{slug} is not in the manifest"))
    }

    /// What the card needs, and the rule for when it is there. Everything a page
    /// says about itself reaches the manifest, and nothing that is not
    /// `included` says anything, exactly as `title` already worked.
    #[tokio::test]
    async fn a_section_carries_what_the_page_says_about_itself() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one/opening, book/one/missing]\n---\n\n# Book\n",
            )
            .page(
                "book/one/opening",
                "---\nsynopsis: They leave, and nobody says why.\nstage: drafted\ntarget: 3000\n---\n\n# Opening\n\nProse here.\n",
            );

        let result = compiled(&wiki, "book").await;

        let opening = find(&result, "book/one/opening");
        assert_eq!(
            opening.synopsis.as_deref(),
            Some("They leave, and nobody says why.")
        );
        assert_eq!(opening.stage.as_deref(), Some("drafted"));
        assert_eq!(opening.target, Some(3000));

        // The root says none of the three, so it reports none of the three.
        let root = find(&result, "book");
        assert_eq!(root.synopsis, None);
        assert_eq!(root.stage, None);
        assert_eq!(root.target, None);

        // And a gap says nothing at all, since there is nothing there to say it.
        let gap = find(&result, "book/one/missing");
        assert_eq!(gap.status, Status::Wanted);
        assert_eq!(gap.title, None);
        assert_eq!(gap.synopsis, None);
        assert_eq!(gap.stage, None);
    }

    /// The number the plan would have missed. A part page's own body is its
    /// heading and its epigraph, so comparing that against a `target` meaning
    /// the whole part would draw it at two per cent forever.
    #[tokio::test]
    async fn a_subtree_is_the_sum_beneath_it_and_words_stay_the_bodys_own() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/two]\n---\n\n# Book\n",
            )
            .page(
                "book/one",
                "---\ntarget: 5000\ncontents: [book/one/opening, book/one/the-ferry]\n---\n\n# Part One\n\n> Four words of epigraph.\n",
            )
            .page("book/one/opening", "# Opening\n\none two three four five\n")
            .page("book/one/the-ferry", "# The Ferry\n\nsix seven eight\n")
            .page("book/two", "# Part Two\n\nnine ten\n");

        let result = compiled(&wiki, "book").await;

        let part = find(&result, "book/one");
        assert_eq!(
            part.words,
            2 + 4,
            "a part's own words are its heading and its epigraph"
        );
        assert_eq!(
            part.subtree,
            (2 + 4) + (1 + 5) + (2 + 3),
            "a part's subtree is everything emitted beneath it"
        );
        assert_eq!(part.target, Some(5000));

        // On a leaf the two numbers are the same, which is what makes the
        // recursive definition one rule rather than two.
        let leaf = find(&result, "book/one/opening");
        assert_eq!(leaf.words, leaf.subtree);

        // And the root's subtree is the whole document.
        assert_eq!(find(&result, "book").subtree, result.words);
    }

    /// A preamble sits at depth zero beside the root rather than above it, so it
    /// must not swallow the book into its own subtree.
    #[tokio::test]
    async fn a_style_page_subtree_is_only_its_own() {
        let wiki = Wiki::new()
            .page("book", "---\ncontents: [book/one]\n---\n\n# Book\n")
            .page("book/one", "# One\n\none two three\n")
            .page("rules/voice", "# Voice\n\nPast tense throughout.\n");

        let result = compile(
            &Slug::parse("book").unwrap(),
            Some(&Slug::parse("rules/voice").unwrap()),
            &wiki,
        )
        .await
        .expect("compiles");

        let style = find(&result, "rules/voice");
        assert_eq!(style.subtree, style.words);
        assert!(find(&result, "book").subtree > 0);
    }

    /// The load-bearing consequence: excluding a part takes its chapters with
    /// it, because a part contributes its heading and dropping only its body
    /// would silently promote its chapters under the previous part. Every one of
    /// them keeps its position in the manifest.
    #[tokio::test]
    async fn excluding_a_part_excludes_everything_under_it_and_keeps_the_positions() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/two]\n---\n\n# Book\n",
            )
            .page(
                "book/one",
                "---\ncompile: false\ncontents: [book/one/opening]\n---\n\n# Part One\n\ncut cut cut\n",
            )
            .page("book/one/opening", "# Opening\n\none two three four\n")
            .page("book/two", "# Part Two\n\nfive six\n");

        let result = compiled(&wiki, "book").await;

        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("book/one", Status::Excluded),
                ("book/one/opening", Status::Excluded),
                ("book/two", Status::Included),
            ],
            "an excluded subtree left the manifest instead of staying in place"
        );

        assert!(!result.markdown.contains("Part One"));
        assert!(!result.markdown.contains("Opening"));
        assert!(result.markdown.contains("Part Two"));

        // Excluded words do not count, in the totals or in any ancestor's
        // subtree. Cutting a chapter moves the book's progress down, and that is
        // the number moving for the right reason.
        assert_eq!(find(&result, "book/one").words, 0);
        assert_eq!(find(&result, "book/one").subtree, 0);
        assert_eq!(result.words, find(&result, "book").subtree);
        assert_eq!(
            result.words,
            1 + (2 + 2),
            "excluded words reached a total: only the root and part two are in"
        );
    }

    /// A chapter that says `compile: false` for itself, with nothing above it
    /// saying anything.
    #[tokio::test]
    async fn a_single_excluded_chapter_leaves_the_rest_of_the_book_alone() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/cut, book/two]\n---\n\n# Book\n",
            )
            .page("book/one", "# One\n")
            .page("book/cut", "---\ncompile: false\n---\n\n# A cut scene\n")
            .page("book/two", "# Two\n");

        let result = compiled(&wiki, "book").await;

        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("book/one", Status::Included),
                ("book/cut", Status::Excluded),
                ("book/two", Status::Included),
            ]
        );
        assert!(!result.markdown.contains("cut scene"));
        // The gap holds a position and no bytes, so it sits exactly where the
        // next chapter picks up and the offsets either side still index what
        // they claim. The separator between sections is added afterwards, which
        // is why this trims: nothing was emitted *at* the gap.
        let cut = find(&result, "book/cut");
        assert_eq!(cut.length, 0);
        assert!(
            result.markdown[cut.offset..]
                .trim_start()
                .starts_with("## Two"),
            "{:?}",
            &result.markdown[cut.offset..]
        );
    }

    /// It interacts with `duplicate` better than expected, and no special case
    /// was needed: the rule is that `duplicate` means already emitted, and
    /// nothing excluded was.
    #[tokio::test]
    async fn an_appendix_under_an_excluded_part_is_included_once_and_duplicate_nowhere() {
        // The excluded part comes first, so the appendix is met while excluded
        // and then met again on a path that includes it.
        let cut_first = Wiki::new()
            .page("book", "---\ncontents: [one, two]\n---\n\n# Book\n")
            .page(
                "one",
                "---\ncompile: false\ncontents: [appendix]\n---\n\n# One\n",
            )
            .page("two", "---\ncontents: [appendix]\n---\n\n# Two\n")
            .page("appendix", "# Appendix\n\nfour words are here\n");

        let result = compiled(&cut_first, "book").await;
        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("one", Status::Excluded),
                ("appendix", Status::Excluded),
                ("two", Status::Included),
                ("appendix", Status::Included),
            ]
        );
        assert_eq!(result.markdown.matches("# Appendix").count(), 1);
        assert!(
            !result
                .sections
                .iter()
                .any(|s| s.status == Status::Duplicate),
            "a page emitted exactly once was reported as a duplicate"
        );

        // The other order, which is the one that would tempt an implementation
        // into reporting `duplicate` for a position that emitted nothing.
        let cut_second = Wiki::new()
            .page("book", "---\ncontents: [one, two]\n---\n\n# Book\n")
            .page("one", "---\ncontents: [appendix]\n---\n\n# One\n")
            .page(
                "two",
                "---\ncompile: false\ncontents: [appendix]\n---\n\n# Two\n",
            )
            .page("appendix", "# Appendix\n\nfour words are here\n");

        assert_eq!(
            statuses(&compiled(&cut_second, "book").await),
            [
                ("book", Status::Included),
                ("one", Status::Included),
                ("appendix", Status::Included),
                ("two", Status::Excluded),
                ("appendix", Status::Excluded),
            ]
        );
    }

    /// Nothing consults `emitted` on the excluded path, so nothing else can end
    /// a loop down there. Without its own guard this walks to the depth limit
    /// and turns a harmless mistake into a refusal.
    #[tokio::test]
    async fn a_cycle_inside_an_excluded_subtree_terminates() {
        let wiki = Wiki::new()
            .page("book", "---\ncontents: [a]\n---\n\n# Book\n")
            .page("a", "---\ncompile: false\ncontents: [b]\n---\n\n# A\n")
            .page("b", "---\ncontents: [a]\n---\n\n# B\n");

        let result = compile(&Slug::parse("book").unwrap(), None, &wiki)
            .await
            .expect("an excluded loop is not a refusal");

        assert_eq!(
            statuses(&result),
            [
                ("book", Status::Included),
                ("a", Status::Excluded),
                ("b", Status::Excluded),
                ("a", Status::Excluded),
            ]
        );
    }

    /// A page listed twice under a parent that is itself excluded. The second
    /// one is excluded rather than duplicate, because nothing was emitted at
    /// either position.
    #[tokio::test]
    async fn the_same_child_twice_under_an_excluded_parent_is_excluded_twice() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncompile: false\ncontents: [one, one]\n---\n\n# Book\n",
            )
            .page("one", "# One\n");

        assert_eq!(
            statuses(&compiled(&wiki, "book").await),
            [
                ("book", Status::Excluded),
                ("one", Status::Excluded),
                ("one", Status::Excluded),
            ]
        );
    }

    /// An exclusion changes what is in the document, so it had better not change
    /// what is in it from one compile to the next.
    #[tokio::test]
    async fn compiling_a_book_with_a_cut_chapter_twice_is_byte_identical() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/cut, book/two]\n---\n\n# Book\n",
            )
            .page("book/one", "# One\n\nFirst.\n")
            .page("book/cut", "---\ncompile: false\n---\n\n# Cut\n")
            .page("book/two", "# Two\n\nSecond.\n");

        assert_eq!(
            compiled(&wiki, "book").await,
            compiled(&wiki, "book").await,
            "two compiles of one book disagreed"
        );
    }

    /// Compiling a page that says it is not compiled. An odd thing to ask for,
    /// and the answer says so rather than pretending otherwise.
    #[tokio::test]
    async fn an_excluded_root_compiles_to_nothing_and_says_why() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncompile: false\ncontents: [book/one]\n---\n\n# Book\n",
            )
            .page("book/one", "# One\n");

        let result = compiled(&wiki, "book").await;

        assert_eq!(result.markdown, "");
        assert_eq!(result.words, 0);
        assert_eq!(
            statuses(&result),
            [("book", Status::Excluded), ("book/one", Status::Excluded)]
        );
    }

    /// Two gaps at two positions, rather than a gap and a `duplicate` pointing
    /// at nothing. `duplicate` means already emitted, and a page nobody has
    /// written was never emitted once.
    #[tokio::test]
    async fn a_chapter_nobody_has_written_is_wanted_at_every_position() {
        let wiki = Wiki::new().page(
            "book",
            "---\ncontents: [book/missing, book/missing]\n---\n\n# Book\n",
        );

        assert_eq!(
            statuses(&compiled(&wiki, "book").await),
            [
                ("book", Status::Included),
                ("book/missing", Status::Wanted),
                ("book/missing", Status::Wanted),
            ]
        );
    }

    // ------------------------------------------------------- heading shift

    #[test]
    fn shifting_by_nothing_changes_nothing() {
        let body = "# One\n\nText.\n";
        assert_eq!(shift_headings(body, 0), body);
    }

    #[test]
    fn shifts_atx_headings_and_leaves_everything_else() {
        assert_eq!(
            shift_headings("# One\n\nText.\n## Two\n", 1),
            "## One\n\nText.\n### Two\n"
        );
        assert_eq!(shift_headings("# One\n", 2), "### One\n");
    }

    /// A `#` inside a fence is not a heading, which is the reason this goes
    /// through the parser at all.
    #[test]
    fn a_hash_inside_a_fence_is_not_shifted() {
        let body = "# Real\n\n```sh\n# a comment\n```\n";
        assert_eq!(
            shift_headings(body, 1),
            "## Real\n\n```sh\n# a comment\n```\n"
        );
    }

    /// Markdown has no seventh level, so a heading that would be pushed past six
    /// stops there rather than becoming a paragraph that starts with hashes.
    #[test]
    fn levels_clamp_at_six() {
        assert_eq!(shift_headings("###### Deep\n", 1), "###### Deep\n");
        assert_eq!(shift_headings("##### Five\n", 3), "###### Five\n");
    }

    /// A setext heading has no marker to shift, so it becomes an ATX one. The
    /// text is kept as written so inline formatting survives.
    #[test]
    fn a_setext_heading_becomes_an_atx_one() {
        assert_eq!(
            shift_headings("Title\n=====\n\nText.\n", 1),
            "## Title\n\nText.\n"
        );
        assert_eq!(shift_headings("Sub\n---\n", 2), "#### Sub\n");
        assert_eq!(
            shift_headings("A *fancy* title\n===\n", 1),
            "## A *fancy* title\n"
        );
    }

    #[test]
    fn crlf_survives_a_shift() {
        assert_eq!(
            shift_headings("# One\r\n\r\nText.\r\n", 1),
            "## One\r\n\r\nText.\r\n"
        );
    }

    #[test]
    fn a_body_with_no_headings_is_untouched() {
        let body = "Just prose, and a [link](a.md).\n";
        assert_eq!(shift_headings(body, 3), body);
    }

    // ------------------------------------------------- who named which entry

    #[tokio::test]
    async fn every_entry_names_the_list_that_named_it_and_where() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/one, book/two]\n---\n\n# Book\n",
            )
            .page("book/one", "---\ncontents: [book/one/a]\n---\n\n# One\n")
            .page("book/one/a", "# A\n")
            .page("book/two", "# Two\n");

        let result = compiled(&wiki, "book").await;
        let named: Vec<(&str, Option<&str>, Option<usize>)> = result
            .sections
            .iter()
            .map(|section| {
                (
                    section.slug.as_str(),
                    section.parent.as_deref(),
                    section.ordinal,
                )
            })
            .collect();

        assert_eq!(
            named,
            vec![
                ("book", None, None),
                ("book/one", Some("book"), Some(0)),
                ("book/one/a", Some("book/one"), Some(0)),
                ("book/two", Some("book"), Some(1)),
            ]
        );
    }

    /// A preamble is a page this request asked for rather than one the spine
    /// reaches, so it is named by nobody and there is nothing to reorder it in.
    #[tokio::test]
    async fn a_preamble_is_named_by_nobody() {
        let wiki = Wiki::new()
            .page("book", "# Book\n")
            .page("rules/voice", "# Voice\n");

        let result = compile(
            &Slug::parse("book").unwrap(),
            Some(&Slug::parse("rules/voice").unwrap()),
            &wiki,
        )
        .await
        .expect("compiles");

        assert_eq!(result.sections[0].slug, "rules/voice");
        assert_eq!(result.sections[0].parent, None);
        assert_eq!(result.sections[0].ordinal, None);
    }

    /// Everything a page would say about itself is dropped on a section that is
    /// not `included`. These two are not about the page, they are about the
    /// entry, and an entry nothing could locate would be an entry nothing could
    /// fix.
    #[tokio::test]
    async fn a_gap_a_repeat_a_bad_entry_and_a_cut_scene_all_say_where_they_sit() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/gone, book/cut, book/live, book/live, '../nope']\n---\n\n# Book\n",
            )
            .page("book/cut", "---\ncompile: false\n---\n\n# Cut\n")
            .page("book/live", "# Live\n");

        let result = compiled(&wiki, "book").await;
        let placed: Vec<(&str, Status, Option<usize>)> = result.sections[1..]
            .iter()
            .map(|section| (section.slug.as_str(), section.status, section.ordinal))
            .collect();

        assert_eq!(
            placed,
            vec![
                ("book/gone", Status::Wanted, Some(0)),
                ("book/cut", Status::Excluded, Some(1)),
                ("book/live", Status::Included, Some(2)),
                ("book/live", Status::Duplicate, Some(3)),
                ("../nope", Status::Invalid, Some(4)),
            ]
        );
        assert!(
            result.sections[1..]
                .iter()
                .all(|section| section.parent.as_deref() == Some("book")),
            "every entry in one list names that list"
        );
    }

    /// The property a client reorders against: what comes back rebuilds the
    /// authored list byte for byte, repeats and typos included. Losing either to
    /// a reorder would be losing something somebody wrote.
    #[tokio::test]
    async fn a_contents_list_is_rebuildable_from_the_manifest() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/gone, book/live, book/live, '../nope', book/cut]\n---\n\n# Book\n",
            )
            .page("book/live", "# Live\n")
            .page("book/cut", "---\ncompile: false\n---\n\n# Cut\n");

        let result = compiled(&wiki, "book").await;

        assert_eq!(
            rebuilt(&result, "book"),
            vec!["book/gone", "book/live", "book/live", "../nope", "book/cut"]
        );
    }

    /// The case that makes counting rows wrong. A page under one excluded part
    /// and one included part is walked down both, so its own children appear
    /// twice carrying the same ordinals, and a rebuild that counted rows would
    /// double the list and write a book with every chapter in it twice.
    #[tokio::test]
    async fn a_parent_walked_down_two_paths_repeats_its_children_with_the_same_ordinals() {
        let wiki = Wiki::new()
            .page(
                "book",
                "---\ncontents: [book/cut, book/live]\n---\n\n# Book\n",
            )
            .page(
                "book/cut",
                "---\ncompile: false\ncontents: [book/shared]\n---\n\n# Cut\n",
            )
            .page("book/live", "---\ncontents: [book/shared]\n---\n\n# Live\n")
            .page(
                "book/shared",
                "---\ncontents: [book/shared/a, book/shared/b]\n---\n\n# Shared\n",
            )
            .page("book/shared/a", "# A\n")
            .page("book/shared/b", "# B\n");

        let result = compiled(&wiki, "book").await;

        let rows = result
            .sections
            .iter()
            .filter(|section| section.parent.as_deref() == Some("book/shared"))
            .count();
        assert_eq!(rows, 4, "twice down the tree, twice in the manifest");

        assert_eq!(
            rebuilt(&result, "book/shared"),
            vec!["book/shared/a", "book/shared/b"],
            "and still one list of two"
        );
    }
}
