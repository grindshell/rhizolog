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
use utoipa::ToSchema;

use crate::frontmatter::{self, FrontmatterError};
use crate::slug::Slug;
use crate::users::Username;

/// Who may read a page.
///
/// A ladder rather than a set of flags, and the ordering is the point: each rung
/// is strictly narrower than the one above it, so "can this account read this
/// page" is one comparison rather than a policy engine.
///
/// The default is [`Visibility::Internal`], which is what an unmarked page means
/// — and unmarked is what every page in an existing wiki is. That choice is why
/// turning authentication on does not silently publish a wiki to the internet,
/// and why it does not silently hide it from the people already using it.
///
/// **None of this is a boundary against whoever holds the disk.** Anybody who
/// can read the wiki directory can read every page in it. See
/// `knowledge-base/visibility.md`.
/// The `description` is set here rather than taken from the doc comment above,
/// for the reason [`crate::slug::Slug`]'s is: that comment is written for
/// somebody reading this file, and it links to Rust items that mean nothing on
/// the wire. What a caller needs is the four words and what each one does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(
    example = "internal",
    description = "Who may read a page.\n\n\
                   - `public` — anyone, including callers who have not signed in. Only \
                   reaches them when the instance sets `RHIZOLOG_ANONYMOUS_READ`; without \
                   that it behaves as `internal`.\n\
                   - `internal` — any account on this wiki. **This is what an unmarked page \
                   means.**\n\
                   - `restricted` — the accounts in `readers`, plus the owner.\n\
                   - `private` — the owner alone.\n\n\
                   An unrecognised word reads as `private` rather than as the default: a typo \
                   in this field must never be the thing that publishes a page.\n\n\
                   On a wiki with no accounts this is inert — there is nobody to keep a page \
                   from. It is also not a boundary against anyone who can read the wiki \
                   directory itself."
)]
pub enum Visibility {
    /// Anyone, including callers who have not signed in.
    ///
    /// Only reaches an anonymous caller when the instance has opted into serving
    /// them at all — see `RHIZOLOG_ANONYMOUS_READ` in [`crate::config`]. Without
    /// that, this behaves as [`Visibility::Internal`], so marking a page public
    /// on a wiki that is not serving the public does nothing.
    Public,
    /// Any account on this wiki. What an unmarked page means.
    #[default]
    Internal,
    /// The `readers` list, plus the owner.
    Restricted,
    /// The owner alone.
    Private,
}

impl Visibility {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Public => "public",
            Self::Internal => "internal",
            Self::Restricted => "restricted",
            Self::Private => "private",
        }
    }

    /// Parse the value as it appears in frontmatter.
    ///
    /// A word nobody recognises is **not** an error and does not fall back to
    /// the default. It reads as [`Visibility::Private`], because the one thing a
    /// typo in this field must never do is publish a page: somebody who wrote
    /// `visibility: privte` was trying to restrict it, and treating that as
    /// "internal" would do the opposite of what they asked.
    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "public" => Self::Public,
            "internal" => Self::Internal,
            "restricted" => Self::Restricted,
            _ => Self::Private,
        }
    }

    /// Whether this page needs an owner to be readable by anybody.
    pub fn needs_owner(self) -> bool {
        matches!(self, Self::Restricted | Self::Private)
    }
}

impl std::fmt::Display for Visibility {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The frontmatter block, exactly as it appears in the file.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frontmatter {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,

    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,

    /// Set once, when the page is created. Absent for files written by hand.
    ///
    /// Accepts a bare `2026-08-19` as well as a full timestamp, because this is
    /// the one field in a page's frontmatter strict enough that writing it the
    /// ordinary way would otherwise make the whole page malformed. It is written
    /// back as a full timestamp — see [`frontmatter::timestamp`].
    #[serde(
        default,
        deserialize_with = "frontmatter::timestamp",
        skip_serializing_if = "Option::is_none"
    )]
    pub created: Option<DateTime<Utc>>,

    /// Who may read this page. Absent means [`Visibility::Internal`].
    ///
    /// Deserialised through a plain `String` rather than straight into the enum
    /// so that an unrecognised word is a *value* rather than a parse failure. A
    /// page whose frontmatter will not parse is reported as malformed and is
    /// invisible in listings, which for a typo in this field would be a strange
    /// way to find out — and would take the rest of the frontmatter with it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub visibility: Option<String>,

    /// The account this page belongs to.
    ///
    /// Set automatically when a page is created or first restricted, to the
    /// account doing it. Required for `restricted` and `private` to mean
    /// anything: a private page with no owner is readable by nobody, which is
    /// the safe direction to fail in but is rarely what anyone wanted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,

    /// Accounts that may read this page when it is `restricted`.
    ///
    /// Ignored for every other visibility, and kept in the file rather than
    /// dropped, so that widening a page and narrowing it again does not lose the
    /// list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub readers: Vec<String>,

    /// A word count to aim at, measured against the **compiled** total from this
    /// page: its own words on a leaf, and the whole work on a page that has
    /// `contents`.
    ///
    /// Accepts `90,000` as well as `90000`, for the reason `created` accepts a
    /// bare date. See [`frontmatter::word_count`].
    #[serde(
        default,
        deserialize_with = "frontmatter::word_count",
        skip_serializing_if = "Option::is_none"
    )]
    pub target: Option<u64>,

    /// The day this page is due, as written.
    ///
    /// Kept as a string rather than parsed into the struct, which is the
    /// opposite of what `created` does, and the difference is what each one is
    /// for. `created` is a timestamp this code writes and compares; `due` is a
    /// note to the author, so the useful behaviour when it says something that
    /// is not a date is the one `owner` has: it names no day, and it does not
    /// take the page down with it. See [`Page::due`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,

    /// The pages this one assembles, in order.
    ///
    /// `None` is a leaf page and `Some([])` is a contents page with nothing in
    /// it yet, which is what a book looks like on the day it is started. The two
    /// are different values and both round-trip, so a `PUT` can set either.
    ///
    /// Entries are **strings**, not [`Slug`]s, and are parsed when the page is
    /// compiled rather than when it is read. A slug that will not parse is one
    /// bad chapter in the manifest; parsing here would make it a malformed page,
    /// which costs the title, the tags and every listing the page appears in.
    /// See `knowledge-base/long-form.md`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub contents: Option<Vec<String>>,
}

impl Frontmatter {
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.tags.is_empty()
            && self.created.is_none()
            && self.visibility.is_none()
            && self.owner.is_none()
            && self.readers.is_empty()
            && self.target.is_none()
            && self.due.is_none()
            && self.contents.is_none()
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
    /// Wrapped rather than restated. These used to be a copy of
    /// [`FrontmatterError`]'s variants with a `From` between them, which is two
    /// places to add a case and two messages to keep in step — the exact thing
    /// [`crate::frontmatter`] exists to avoid. [`crate::times::TimeError`] was
    /// already doing it this way.
    #[error(transparent)]
    Frontmatter(#[from] FrontmatterError),
}

impl Page {
    /// Parse the text of a page file.
    pub fn from_markdown(
        slug: Slug,
        text: &str,
        updated: DateTime<Utc>,
    ) -> Result<Self, PageError> {
        // Taken before the BOM is stripped: `text` is the file's contents
        // verbatim, so its length is the file's length, and that is what the
        // startup scan compares against the filesystem.
        let size = text.len() as u64;

        let text = frontmatter::strip_bom(text);

        let (frontmatter, body) = match frontmatter::split(text)? {
            Some((yaml, body)) => (frontmatter::parse(yaml)?, body),
            None => (Frontmatter::default(), text),
        };

        Ok(Self {
            slug,
            frontmatter,
            body: body.to_owned(),
            updated,
            size,
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

        frontmatter::compose(&yaml, &self.body)
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

    /// Whether [`Page::title`] came from the frontmatter rather than being
    /// derived from the body's first heading or the slug.
    ///
    /// A blank frontmatter title counts as absent, matching [`Page::title`].
    pub fn has_stored_title(&self) -> bool {
        self.frontmatter
            .title
            .as_ref()
            .is_some_and(|title| !title.trim().is_empty())
    }

    pub fn tags(&self) -> &[String] {
        &self.frontmatter.tags
    }

    /// When the page was created, defaulting to its mtime for files that were
    /// written by hand and never carried the field.
    pub fn created(&self) -> DateTime<Utc> {
        self.frontmatter.created.unwrap_or(self.updated)
    }

    /// Who may read this page.
    ///
    /// An absent field is [`Visibility::Internal`] — any account — and an
    /// unrecognised one is [`Visibility::Private`]. See [`Visibility::parse`]
    /// for why a typo fails closed rather than falling back to the default.
    pub fn visibility(&self) -> Visibility {
        match &self.frontmatter.visibility {
            Some(raw) => Visibility::parse(raw),
            None => Visibility::default(),
        }
    }

    /// The account this page belongs to, if the name is one that could exist.
    ///
    /// An owner that will not parse as a username names nobody, so it is treated
    /// as absent rather than as a value nothing will ever match. The distinction
    /// only matters for the message a reader gets, since neither is readable.
    pub fn owner(&self) -> Option<Username> {
        self.frontmatter
            .owner
            .as_deref()
            .and_then(|raw| Username::parse(raw).ok())
    }

    /// How many words the body holds.
    ///
    /// The count an editor means rather than the one `wc -w` gives: see
    /// [`crate::markdown::count_words`] for every rule and why each is there.
    pub fn words(&self) -> u64 {
        crate::markdown::count_words(&self.body)
    }

    /// The day this page is due, if what the field says is one.
    ///
    /// A bare `2027-03-01` means midnight UTC, and a full timestamp is taken as
    /// written. A wall-clock time with no zone is refused for
    /// [`frontmatter::timestamp`]'s reason: it carries a real time whose meaning
    /// depends on where it was written.
    ///
    /// Anything else names no day, exactly as an `owner` that is not a username
    /// names nobody. The field stays in the file either way, so a typo is
    /// visible and fixable rather than silently discarded on the next write.
    pub fn due(&self) -> Option<DateTime<Utc>> {
        self.frontmatter.due.as_deref().and_then(frontmatter::day)
    }

    /// The pages this one assembles, in order, as written.
    ///
    /// Not parsed into slugs here. See [`Frontmatter::contents`].
    pub fn contents(&self) -> Option<&[String]> {
        self.frontmatter.contents.as_deref()
    }

    /// The accounts named in `readers`, ignoring any that are not valid names.
    ///
    /// Only meaningful when [`Page::visibility`] is [`Visibility::Restricted`];
    /// the list is kept on other pages rather than dropped, so narrowing a page
    /// again does not lose it.
    pub fn readers(&self) -> Vec<Username> {
        self.frontmatter
            .readers
            .iter()
            .filter_map(|raw| Username::parse(raw).ok())
            .collect()
    }
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

    /// Every string field in the frontmatter goes out through a YAML emitter and
    /// comes back through a YAML parser, and YAML has a long list of plain
    /// scalars that mean something other than themselves. A title of `123`, a
    /// tag of `no`, an owner called `null`: each is a word somebody may
    /// reasonably type, and each is a word the format spells differently.
    ///
    /// Nothing here needs the *emitted* form to be any particular thing — only
    /// that what went in comes back. Asserting the quoting instead would be
    /// asserting `serde_yaml_ng`'s style, which is free to change and is not the
    /// property that matters.
    #[test]
    fn every_yaml_reserved_word_survives_a_round_trip() {
        let hazards = [
            // Nulls, booleans, and the YAML 1.1 booleans that are not YAML 1.2's
            // — `no` for Norway is the famous one.
            "null",
            "Null",
            "NULL",
            "~",
            "true",
            "false",
            "True",
            "yes",
            "no",
            "on",
            "off",
            "y",
            "n", // Numbers, in every base and shape YAML resolves.
            "123",
            "0",
            "007",
            "0x1f",
            "0b101",
            "0o17",
            "1_000",
            "1e3",
            "inf",
            "nan",
            // Timestamps and the sexagesimals YAML 1.1 reads as numbers.
            "2026-08-19",
            "12:30:00",
            "1:30",
            // Indicators: characters that start something in YAML's grammar.
            ".inf",
            ".nan",
            "-",
            "--",
            "---",
            "...",
            "#hash",
            "key: value",
            "[a, b]",
            "{a: b}",
            "*anchor",
            "&anchor",
            "!tag",
            "%directive",
            "@at",
            "`tick",
            "|pipe",
            ">fold",
            // Whitespace, emptiness, and the two kinds of quote.
            " leading",
            "trailing ",
            "",
            "a\nb",
            "\ttab",
            "'quoted'",
            "\"quoted\"",
        ];

        for hazard in hazards {
            let mut page = page("Body.\n");
            page.frontmatter.title = Some(hazard.to_owned());
            page.frontmatter.tags = vec![hazard.to_owned()];
            page.frontmatter.owner = Some(hazard.to_owned());
            page.frontmatter.readers = vec![hazard.to_owned()];

            let written = page.to_markdown();
            let read = Page::from_markdown(
                Slug::parse("notes/rhizome").unwrap(),
                &written,
                at("2026-08-05T12:00:00Z"),
            )
            .unwrap_or_else(|error| panic!("{hazard:?} would not parse back: {error}\n{written}"));

            assert_eq!(
                read.frontmatter, page.frontmatter,
                "{hazard:?} did not survive the round trip:\n{written}"
            );
        }
    }

    /// The date somebody writes by hand, and what happens to it afterwards.
    ///
    /// Rewriting normalises it to a full timestamp, which is worth stating
    /// because this module otherwise keeps the frontmatter as written. `created`
    /// is not stored as written and never was — it is parsed into a
    /// `DateTime<Utc>`, so an offset was already being normalised away. Midnight
    /// is what the bare date meant; the file just says so afterwards.
    #[test]
    fn a_bare_date_is_read_as_midnight_and_written_back_in_full() {
        let page = page("---\ntitle: Rhizome\ncreated: 2026-08-19\n---\n\nBody.\n");

        assert_eq!(page.created(), at("2026-08-19T00:00:00Z"));

        let written = page.to_markdown();
        assert!(
            written.contains("created: 2026-08-19T00:00:00Z"),
            "the rewritten page did not carry a full timestamp:\n{written}"
        );

        let again = Page::from_markdown(
            Slug::parse("notes/rhizome").unwrap(),
            &written,
            at("2026-08-05T12:00:00Z"),
        )
        .expect("the rewritten page parses");
        assert_eq!(again.created(), page.created());
    }

    /// The whole reason the bare date is worth accepting: the cost of refusing
    /// it was never the field, it was the page.
    #[test]
    fn a_date_written_by_hand_does_not_cost_the_page_its_title() {
        let page = page("---\ntitle: Rhizome\ntags: [theory]\ncreated: 2026-08-19\n---\n\nBody.\n");

        assert_eq!(page.title(), "Rhizome");
        assert_eq!(page.tags(), ["theory"]);

        // And something that is not a date at all still is refused, because at
        // that point there is nothing to be faithful to.
        assert!(
            Page::from_markdown(
                Slug::parse("notes/rhizome").unwrap(),
                "---\ncreated: yes\n---\n\nBody.\n",
                at("2026-08-05T12:00:00Z"),
            )
            .is_err()
        );
    }

    /// The one YAML word that is not a string, and what it means in each place.
    ///
    /// `null`, `~` and an empty value are the *scalar*, so a field holding one is
    /// a field with nothing in it. That is the right reading for all three
    /// optional fields, and it is worth stating because each fails in a
    /// different direction: a missing title is derived, a missing owner is
    /// nobody, and a missing visibility is the default rather than the
    /// unrecognised-word rule — `null` never becomes a word for that rule to
    /// fail closed on.
    ///
    /// Inside a list the same token is four characters of text instead, which is
    /// an asymmetry rather than a hazard: `null` is a legal username, and `~` is
    /// not a username at all, so it names nobody.
    #[test]
    fn an_explicit_yaml_null_is_an_absent_field() {
        let cleared = page("---\ntitle: null\nowner: ~\nvisibility:\n---\n\nBody.\n");

        assert_eq!(cleared.frontmatter.title, None);
        assert_eq!(cleared.frontmatter.owner, None);
        assert_eq!(cleared.frontmatter.visibility, None);
        assert_eq!(cleared.visibility(), Visibility::Internal);
        assert_eq!(cleared.owner(), None);

        let listed = page("---\ntags: [null]\nreaders: [null, ~]\n---\n\nBody.\n");

        assert_eq!(listed.frontmatter.tags, ["null"]);
        assert_eq!(listed.frontmatter.readers, ["null", "~"]);
        // `~` is not a username, so it is dropped rather than kept as a reader
        // nobody can ever be.
        assert_eq!(
            listed.readers(),
            [Username::parse("null").expect("null is a username")]
        );
    }

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

    /// Found by hand-editing a page against a running server: PowerShell's
    /// `Set-Content -Encoding utf8` writes a BOM, and the title came back as
    /// the slug's basename because the frontmatter had been read as body.
    #[test]
    fn a_utf8_bom_does_not_hide_the_frontmatter() {
        let parsed = page("\u{feff}---\ntitle: Rhizome\ntags: [theory]\n---\n\nBranches off.\n");

        assert_eq!(parsed.frontmatter.title.as_deref(), Some("Rhizome"));
        assert_eq!(parsed.tags(), ["theory"]);
        assert_eq!(parsed.body, "\nBranches off.\n");
        assert_eq!(parsed.title(), "Rhizome");
    }

    #[test]
    fn a_utf8_bom_is_stripped_from_a_body_with_no_frontmatter() {
        let parsed = page("\u{feff}# Heading\n\nBody.\n");

        assert_eq!(parsed.body, "# Heading\n\nBody.\n");
        assert_eq!(parsed.title(), "Heading");
    }

    /// The BOM counts toward the file's length even though it is not part of
    /// the body — otherwise every scan would see a size mismatch and reindex
    /// the page forever.
    #[test]
    fn a_bom_counts_toward_the_recorded_size() {
        let text = "\u{feff}Body.\n";
        assert_eq!(page(text).size, text.len() as u64);
        assert_eq!(page(text).size, 9, "3 bytes of BOM plus 6 of body");
    }

    /// A BOM is not written back. This changes the file's bytes on the next
    /// write, which is intended: one canonical encoding beats two.
    #[test]
    fn a_bom_is_not_written_back() {
        let rendered = page("\u{feff}---\ntitle: Rhizome\n---\nBody.\n").to_markdown();

        assert!(!rendered.starts_with('\u{feff}'));
        assert_eq!(rendered, "---\ntitle: Rhizome\n---\nBody.\n");
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

        assert!(matches!(
            result,
            Err(PageError::Frontmatter(FrontmatterError::Unterminated))
        ));
    }

    /// The two ways a frontmatter block can be refused are told apart, because
    /// they send a reader to different places: one to their punctuation, the
    /// other to a value that is the wrong kind of thing.
    #[test]
    fn a_syntax_error_and_an_unreadable_value_are_different_errors() {
        let broken_yaml = Page::from_markdown(
            Slug::parse("notes/rhizome").unwrap(),
            "---\ntitle: [a, b\n---\n\nBody.\n",
            at("2026-08-05T12:00:00Z"),
        );
        assert!(
            matches!(
                broken_yaml,
                Err(PageError::Frontmatter(FrontmatterError::InvalidYaml(_)))
            ),
            "{broken_yaml:?}"
        );

        let bad_value = Page::from_markdown(
            Slug::parse("notes/rhizome").unwrap(),
            "---\ncreated: tomorrow\n---\n\nBody.\n",
            at("2026-08-05T12:00:00Z"),
        );
        assert!(
            matches!(
                bad_value,
                Err(PageError::Frontmatter(FrontmatterError::UnreadableValue(_)))
            ),
            "{bad_value:?}"
        );

        let message = bad_value.unwrap_err().to_string();
        assert!(
            !message.contains("not valid YAML"),
            "a page whose YAML parsed was told its YAML is invalid: {message}"
        );
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

    /// The rule the whole `contents` design rests on: a chapter somebody
    /// mistyped is one bad entry, never a page that has fallen out of the wiki.
    /// Parsing these into slugs here is what would break it, which is why they
    /// are strings.
    #[test]
    fn a_contents_entry_that_is_not_a_slug_does_not_cost_the_page() {
        let parsed = page(
            "---\ntitle: The Long Way Round\ntags: [manuscript]\ncontents:\n  - book/one\n  - ../etc/passwd\n  - CON\n  - \"\"\n---\n\nBody.\n",
        );

        assert_eq!(parsed.title(), "The Long Way Round");
        assert_eq!(parsed.tags(), ["manuscript"]);
        assert_eq!(
            parsed.contents(),
            Some(
                ["book/one", "../etc/passwd", "CON", ""]
                    .map(String::from)
                    .as_slice()
            ),
            "entries are kept exactly as written, for compile to judge"
        );
    }

    /// Absent is an ordinary page; `[]` is a manuscript with no chapters yet.
    /// Both have to survive a write, or "start a book" is unexpressible.
    #[test]
    fn an_absent_contents_and_an_empty_one_both_round_trip() {
        let ordinary = page("---\ntitle: Ordinary\n---\n\nBody.\n");
        assert_eq!(ordinary.contents(), None);
        assert!(
            !ordinary.to_markdown().contains("contents"),
            "an ordinary page grew a contents line"
        );

        let started = page("---\ntitle: Started\ncontents: []\n---\n\nBody.\n");
        assert_eq!(started.contents(), Some([].as_slice()));

        let written = started.to_markdown();
        assert!(written.contains("contents: []"), "{written}");
        let again = page(&written);
        assert_eq!(again.frontmatter, started.frontmatter);
    }

    /// The same argument the bare date makes. `90,000` is how a person writes a
    /// word count, and refusing it would cost them the page rather than the
    /// field.
    #[test]
    fn a_target_written_the_ordinary_way_is_read() {
        for text in ["90000", "90,000", "90_000", "90 000"] {
            let parsed = page(&format!(
                "---\ntitle: Book\ntarget: \"{text}\"\n---\n\nBody.\n"
            ));
            assert_eq!(parsed.frontmatter.target, Some(90_000), "{text:?}");
        }

        // A plain integer is the spelling this writes, and it is written back as
        // one whatever it arrived as.
        let plain = page("---\ntarget: 90000\n---\n\nBody.\n");
        assert_eq!(plain.frontmatter.target, Some(90_000));
        assert!(plain.to_markdown().contains("target: 90000"));
    }

    /// A negative target is not a small one. Reading it as zero would report a
    /// page as finished, and there is no honest value to fall back to.
    #[test]
    fn a_target_that_is_not_a_count_is_refused() {
        for text in ["-1", "lots", "9.5", "1e3"] {
            let result = Page::from_markdown(
                Slug::parse("book").unwrap(),
                &format!("---\ntarget: {text}\n---\n\nBody.\n"),
                at("2026-08-05T12:00:00Z"),
            );
            assert!(result.is_err(), "{text:?} was accepted as a word count");
        }
    }

    /// `due` is a note to the author, so it behaves like `owner` rather than
    /// like `created`: a value that is not a date names no day and leaves the
    /// page alone. The field stays in the file, so the mistake is visible.
    #[test]
    fn a_due_date_that_is_not_a_date_names_no_day_and_keeps_the_page() {
        let bare = page("---\ndue: 2027-03-01\n---\n\nBody.\n");
        assert_eq!(bare.due(), Some(at("2027-03-01T00:00:00Z")));

        let full = page("---\ndue: 2027-03-01T09:30:00Z\n---\n\nBody.\n");
        assert_eq!(full.due(), Some(at("2027-03-01T09:30:00Z")));

        let nonsense = page("---\ntitle: Book\ndue: soon\n---\n\nBody.\n");
        assert_eq!(nonsense.due(), None);
        assert_eq!(nonsense.title(), "Book", "a bad date took the page with it");
        assert!(
            nonsense.to_markdown().contains("due: soon"),
            "the field was silently dropped rather than left to be fixed"
        );

        // A wall-clock time with no zone is refused for the reason `created`
        // refuses it: reading it as UTC would move it by up to fourteen hours.
        assert_eq!(
            page("---\ndue: 2027-03-01T09:30:00\n---\n\nB.\n").due(),
            None
        );
    }

    #[test]
    fn the_new_fields_do_not_make_an_empty_frontmatter_look_written() {
        assert!(Frontmatter::default().is_empty());
        assert!(
            !Frontmatter {
                target: Some(1),
                ..Default::default()
            }
            .is_empty()
        );
        assert!(
            !Frontmatter {
                due: Some("2027-03-01".into()),
                ..Default::default()
            }
            .is_empty()
        );
        assert!(
            !Frontmatter {
                contents: Some(Vec::new()),
                ..Default::default()
            }
            .is_empty()
        );
    }
}
