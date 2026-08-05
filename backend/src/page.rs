//! The on-disk page format: YAML frontmatter followed by a markdown body.
//!
//! ```markdown
//! ---
//! title: Rhizome
//! tags: [theory, deleuze]
//! created: 2026-08-05T10:00:00Z
//! ---
//!
//! Knowledge branches off chaotically. See [[notes/rust/async]].
//! ```
//!
//! [`Page`] stays faithful to what is in the file — the frontmatter is stored
//! as written so that rewriting a page does not invent fields the author never
//! set. Resolved values (the title after its fallback chain, `created` after
//! its default) are exposed through accessors instead.
//!
//! Note the absence of an `updated` field. It is read from the file's mtime.
//! Storing it in frontmatter would mean every hand-edit and every
//! `git checkout` left it lying, and files are the source of truth here.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::slug::Slug;

const DELIMITER: &str = "---";

/// The frontmatter block, exactly as it appears in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Set once, when the page is created. Absent for files written by hand.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub created: Option<DateTime<Utc>>,
}

impl Frontmatter {
    pub fn is_empty(&self) -> bool {
        self.title.is_none() && self.tags.is_empty() && self.created.is_none()
    }
}

/// A page, as parsed from a file on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub slug: Slug,
    pub frontmatter: Frontmatter,
    pub body: String,
    /// The file's mtime. Not part of the file's contents.
    pub updated: DateTime<Utc>,
    /// The file's length in bytes. Not part of the file's contents.
    ///
    /// This must be the length of the file as it sits on disk, not of anything
    /// re-serialised from [`Page::to_markdown`] — the startup scan compares it
    /// against what the walker reports, and a hand-written file whose YAML is
    /// formatted differently to ours would otherwise look changed on every
    /// single startup.
    pub size: u64,
}

#[derive(Debug, Error)]
pub enum PageError {
    #[error("frontmatter opens with `---` but is never closed")]
    UnterminatedFrontmatter,

    #[error("frontmatter is not valid YAML: {0}")]
    InvalidFrontmatter(#[from] serde_yaml_ng::Error),
}

impl Page {
    /// Parse the text of a page file.
    pub fn from_markdown(
        slug: Slug,
        text: &str,
        updated: DateTime<Utc>,
    ) -> Result<Self, PageError> {
        let (frontmatter, body) = match split_frontmatter(text)? {
            Some((yaml, body)) => (parse_frontmatter(yaml)?, body),
            None => (Frontmatter::default(), text),
        };

        Ok(Self {
            slug,
            frontmatter,
            body: body.to_owned(),
            updated,
            // `text` is the file's contents verbatim, so its length is the
            // file's length.
            size: text.len() as u64,
        })
    }

    /// Render the page back to the text that belongs in its file.
    pub fn to_markdown(&self) -> String {
        if self.frontmatter.is_empty() {
            return self.body.clone();
        }

        // Serialising a struct with only skip-if-empty fields left cannot fail,
        // and `is_empty` above has already established at least one is present.
        let yaml = serde_yaml_ng::to_string(&self.frontmatter)
            .expect("frontmatter is a plain struct of strings and timestamps");

        let mut out = String::with_capacity(yaml.len() + self.body.len() + 16);
        out.push_str(DELIMITER);
        out.push('\n');
        out.push_str(&yaml);
        if !yaml.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(DELIMITER);
        out.push('\n');
        out.push_str(&self.body);
        out
    }

    /// The page's display title.
    ///
    /// Falls back from explicit frontmatter, to the first level-one heading in
    /// the body, to the slug's final segment with separators turned into
    /// spaces.
    pub fn title(&self) -> String {
        if let Some(title) = &self.frontmatter.title
            && !title.trim().is_empty()
        {
            return title.trim().to_owned();
        }
        if let Some(heading) = first_heading(&self.body) {
            return heading;
        }
        humanize(self.slug.basename())
    }

    pub fn tags(&self) -> &[String] {
        &self.frontmatter.tags
    }

    /// When the page was created, defaulting to its mtime for files that were
    /// written by hand and never carried the field.
    pub fn created(&self) -> DateTime<Utc> {
        self.frontmatter.created.unwrap_or(self.updated)
    }
}

/// Split leading frontmatter from the body.
///
/// Returns `Ok(None)` when the text does not open with a frontmatter fence at
/// all, which is the ordinary case for a hand-written file.
fn split_frontmatter(text: &str) -> Result<Option<(&str, &str)>, PageError> {
    let Some(after_open) = text.strip_prefix(DELIMITER).and_then(strip_one_line_ending) else {
        return Ok(None);
    };

    let mut offset = 0;
    for line in after_open.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == DELIMITER {
            let yaml = &after_open[..offset];
            let body = &after_open[offset + line.len()..];
            return Ok(Some((yaml, body)));
        }
        offset += line.len();
    }

    Err(PageError::UnterminatedFrontmatter)
}

fn parse_frontmatter(yaml: &str) -> Result<Frontmatter, PageError> {
    // An empty block is legal and means "no fields", but YAML parses the empty
    // document as null rather than as an empty mapping.
    if yaml.trim().is_empty() {
        return Ok(Frontmatter::default());
    }
    Ok(serde_yaml_ng::from_str(yaml)?)
}

fn strip_one_line_ending(text: &str) -> Option<&str> {
    text.strip_prefix("\r\n")
        .or_else(|| text.strip_prefix('\n'))
}

/// The first ATX level-one heading, skipping fenced code blocks so that a `#`
/// comment inside an example does not become the page title.
fn first_heading(body: &str) -> Option<String> {
    let mut fence: Option<&str> = None;

    for line in body.lines() {
        let trimmed = line.trim_start();

        match fence {
            Some(marker) => {
                if trimmed.starts_with(marker) {
                    fence = None;
                }
                continue;
            }
            None => {
                if trimmed.starts_with("```") {
                    fence = Some("```");
                    continue;
                }
                if trimmed.starts_with("~~~") {
                    fence = Some("~~~");
                    continue;
                }
            }
        }

        if let Some(heading) = trimmed.strip_prefix("# ") {
            let heading = heading.trim().trim_end_matches('#').trim();
            if !heading.is_empty() {
                return Some(heading.to_owned());
            }
        }
    }

    None
}

/// `rust-async_io` becomes `Rust async io`.
///
/// Only the first character is capitalised. Title-casing every word reads
/// worse than it sounds — `io` becomes `Io`, `api` becomes `Api` — and this is
/// only a last-resort fallback for a page with neither a title nor a heading.
fn humanize(basename: &str) -> String {
    let spaced: String = basename
        .chars()
        .map(|c| if c == '-' || c == '_' { ' ' } else { c })
        .collect();

    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
        None => spaced,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc)
    }

    fn page(text: &str) -> Page {
        Page::from_markdown(
            Slug::parse("notes/rhizome").unwrap(),
            text,
            at("2026-08-05T12:00:00Z"),
        )
        .expect("page should parse")
    }

    #[test]
    fn parses_frontmatter_and_body() {
        let parsed = page(
            "---\ntitle: Rhizome\ntags: [theory, deleuze]\ncreated: 2026-08-05T10:00:00Z\n---\n\nBranches off.\n",
        );

        assert_eq!(parsed.frontmatter.title.as_deref(), Some("Rhizome"));
        assert_eq!(parsed.tags(), ["theory", "deleuze"]);
        assert_eq!(parsed.created(), at("2026-08-05T10:00:00Z"));
        assert_eq!(parsed.body, "\nBranches off.\n");
    }

    #[test]
    fn handles_a_file_with_no_frontmatter() {
        let parsed = page("# Rhizome\n\nBranches off.\n");

        assert_eq!(parsed.frontmatter, Frontmatter::default());
        assert_eq!(parsed.body, "# Rhizome\n\nBranches off.\n");
        // With no explicit `created`, the mtime stands in.
        assert_eq!(parsed.created(), at("2026-08-05T12:00:00Z"));
    }

    /// Windows editors write CRLF, and this all runs on Windows.
    #[test]
    fn handles_crlf_line_endings() {
        let parsed = page("---\r\ntitle: Rhizome\r\n---\r\n\r\nBranches off.\r\n");

        assert_eq!(parsed.frontmatter.title.as_deref(), Some("Rhizome"));
        assert_eq!(parsed.body, "\r\nBranches off.\r\n");
    }

    #[test]
    fn empty_frontmatter_block_is_legal() {
        let parsed = page("---\n---\nBody.\n");

        assert_eq!(parsed.frontmatter, Frontmatter::default());
        assert_eq!(parsed.body, "Body.\n");
    }

    #[test]
    fn rejects_unterminated_frontmatter() {
        let result = Page::from_markdown(
            Slug::parse("notes/rhizome").unwrap(),
            "---\ntitle: Rhizome\n\nBranches off.\n",
            at("2026-08-05T12:00:00Z"),
        );

        assert!(matches!(result, Err(PageError::UnterminatedFrontmatter)));
    }

    /// A horizontal rule in the body must not be mistaken for a fence.
    #[test]
    fn a_body_that_merely_contains_a_rule_is_not_frontmatter() {
        let parsed = page("Some text.\n\n---\n\nMore text.\n");

        assert_eq!(parsed.frontmatter, Frontmatter::default());
        assert_eq!(parsed.body, "Some text.\n\n---\n\nMore text.\n");
    }

    #[test]
    fn round_trips_unchanged() {
        for text in [
            "---\ntitle: Rhizome\ntags:\n- theory\n- deleuze\ncreated: 2026-08-05T10:00:00Z\n---\n\nBranches off.\n",
            "# Just a body\n\nNo frontmatter here.\n",
            "---\ntitle: Only a title\n---\nBody.\n",
        ] {
            assert_eq!(
                page(text).to_markdown(),
                text,
                "round trip changed the file"
            );
        }
    }

    #[test]
    fn title_prefers_frontmatter() {
        let parsed = page("---\ntitle: From frontmatter\n---\n\n# From heading\n");
        assert_eq!(parsed.title(), "From frontmatter");
    }

    #[test]
    fn title_falls_back_to_the_first_heading() {
        assert_eq!(page("# From heading\n\nBody.\n").title(), "From heading");
        assert_eq!(page("Intro.\n\n# Later heading\n").title(), "Later heading");
        // Closing hashes are decoration, not content.
        assert_eq!(page("# Closed heading #\n").title(), "Closed heading");
    }

    /// A `#` comment in an example is not the page's title.
    #[test]
    fn headings_inside_code_fences_are_ignored() {
        assert_eq!(
            page("```sh\n# cargo run\n```\n\n# Real heading\n").title(),
            "Real heading"
        );
        assert_eq!(
            page("~~~\n# not a title\n~~~\n\n# Real heading\n").title(),
            "Real heading"
        );
        // A fenced block with nothing after it leaves no heading at all.
        assert_eq!(page("```\n# cargo run\n```\n").title(), "Rhizome");
    }

    #[test]
    fn title_falls_back_to_the_humanized_basename() {
        let parsed = Page::from_markdown(
            Slug::parse("notes/rust-async_io").unwrap(),
            "Body with no heading.\n",
            at("2026-08-05T12:00:00Z"),
        )
        .unwrap();

        assert_eq!(parsed.title(), "Rust async io");
    }

    /// Levels other than one are not titles, and neither is `#foo`.
    #[test]
    fn ignores_things_that_are_not_level_one_headings() {
        assert_eq!(page("## Subheading\n").title(), "Rhizome");
        assert_eq!(page("#nospace\n").title(), "Rhizome");
        assert_eq!(page("#\n").title(), "Rhizome");
    }

    #[test]
    fn writes_no_frontmatter_when_there_is_none_to_write() {
        let parsed = page("Just a body.\n");
        assert_eq!(parsed.to_markdown(), "Just a body.\n");
    }
}
