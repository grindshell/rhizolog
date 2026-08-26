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
}

impl Status {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Included => "included",
            Self::Wanted => "wanted",
            Self::Invalid => "invalid",
            Self::Duplicate => "duplicate",
            Self::Unreadable => "unreadable",
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
    /// How many contents lists deep this page sits. The root is zero.
    pub depth: usize,
    pub words: u64,
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

    if let Some(style) = style
        && let Fetched::Page(page) = pages.fetch(style).await
    {
        emit(&mut out, &mut sections, &mut words, &page, 0)?;
    }

    // An explicit stack rather than recursion, which for an async walk would
    // mean boxing every level. Children are pushed in reverse so they come off
    // in the order the contents list names them.
    let mut stack: Vec<(String, usize)> = vec![(root.to_string(), 0)];
    let mut emitted: HashSet<String> = HashSet::new();

    while let Some((raw, depth)) = stack.pop() {
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
            sections.push(gap(raw, depth, out.len(), Status::Invalid));
            continue;
        };

        if !emitted.insert(slug.to_string()) {
            sections.push(gap(raw, depth, out.len(), Status::Duplicate));
            continue;
        }

        match pages.fetch(&slug).await {
            Fetched::Missing => sections.push(gap(raw, depth, out.len(), Status::Wanted)),
            Fetched::Unreadable => sections.push(gap(raw, depth, out.len(), Status::Unreadable)),
            Fetched::Page(page) => {
                emit(&mut out, &mut sections, &mut words, &page, depth)?;

                for child in page.contents().unwrap_or_default().iter().rev() {
                    stack.push((child.clone(), depth + 1));
                }
            }
        }
    }

    Ok(Compiled {
        markdown: out,
        sections,
        words,
        target,
    })
}

/// Append one page's body and record where it landed.
fn emit(
    out: &mut String,
    sections: &mut Vec<Section>,
    words: &mut u64,
    page: &Page,
    depth: usize,
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

    sections.push(Section {
        slug: page.slug.to_string(),
        title: Some(page.title()),
        depth,
        words: counted,
        offset,
        length: body.len(),
        status: Status::Included,
    });

    Ok(())
}

fn gap(slug: String, depth: usize, offset: usize, status: Status) -> Section {
    Section {
        slug,
        title: None,
        depth,
        words: 0,
        offset,
        length: 0,
        status,
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
}
