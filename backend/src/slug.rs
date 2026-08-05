//! Page identity.
//!
//! A slug is a page's path relative to the wiki root with the `.md` extension
//! removed, always using `/` as the separator: `notes/rust/async.md` has the
//! slug `notes/rust/async`.
//!
//! This module is the **only** sanctioned way to turn untrusted input into a
//! filesystem path. Slugs arrive from HTTP; a slug that escapes the wiki root
//! is arbitrary file read and write. Everything that touches the filesystem
//! goes through [`Slug::to_path`], and [`Slug::parse`] is the gate.
//!
//! Development happens on Windows, which accepts several things Linux would
//! reject — trailing dots and spaces are silently stripped, `CON` is a device
//! no matter what directory it lives in, and `foo:bar` opens an alternate data
//! stream. The validation below rejects all of them on every platform so that a
//! wiki authored on one OS stays valid on the other.

use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use thiserror::Error;
use utoipa::ToSchema;

/// Maximum length of a whole slug, in bytes.
pub const MAX_SLUG_LEN: usize = 512;

/// Maximum length of a single path segment, in bytes. Most filesystems cap
/// individual components at 255.
pub const MAX_SEGMENT_LEN: usize = 250; // 255 less room for the `.md` suffix

/// Names that name a device rather than a file on Windows, regardless of
/// extension or containing directory. `CON.md` is still the console.
const RESERVED_STEMS: [&str; 24] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Characters that are illegal in a Windows filename. `:` is the dangerous one
/// — it introduces both drive prefixes (`C:`) and alternate data streams.
const FORBIDDEN_CHARS: [char; 7] = ['<', '>', ':', '"', '|', '?', '*'];

/// A validated page identifier.
///
/// The only way to construct one is [`Slug::parse`], so holding a `Slug` is
/// proof that the invariants below hold.
/// The `description` is set explicitly rather than taken from the doc comment
/// above. That comment is for people reading this file — it talks about
/// `Slug::parse` and about holding the type — and none of it means anything to
/// somebody reading the OpenAPI document, where a rustdoc link is just a dead
/// reference. What a caller needs is the format and the rules.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, ToSchema)]
#[schema(
    value_type = String,
    example = "notes/rust/async",
    description = "A page's identifier: its path under the wiki root, `/`-separated, \
                   without the `.md` extension. `notes/rust/async.md` is \
                   `notes/rust/async`.\n\n\
                   Slugs must not begin or end with `/`, contain an empty segment, \
                   `.`, `..`, a backslash, or any of `< > : \" | ? *`. No segment may \
                   begin or end with a dot or whitespace, or be a reserved Windows \
                   device name such as `CON` or `LPT1` — a wiki written on one \
                   operating system stays valid on the other.\n\n\
                   A rejected slug comes back as a `400` whose `details.rule` names \
                   which of these was broken."
)]
pub struct Slug(String);

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SlugError {
    #[error("slug is empty")]
    Empty,

    #[error("slug is longer than {MAX_SLUG_LEN} bytes")]
    TooLong,

    #[error("path segment {segment:?} is longer than {MAX_SEGMENT_LEN} bytes")]
    SegmentTooLong { segment: String },

    #[error("slug must not begin or end with '/'")]
    LeadingOrTrailingSlash,

    #[error("slug must not contain an empty path segment")]
    EmptySegment,

    #[error("path segment {segment:?} is a relative path component")]
    RelativeSegment { segment: String },

    #[error("slug must not contain a backslash; use '/' to separate path segments")]
    Backslash,

    #[error("slug must not contain the character {character:?}")]
    ForbiddenCharacter { character: char },

    #[error("path segment {segment:?} must not begin or end with a dot or whitespace")]
    SegmentPadding { segment: String },

    #[error("path segment {segment:?} is a reserved device name on Windows")]
    ReservedName { segment: String },
}

impl SlugError {
    /// A stable, machine-readable identifier for this rule.
    ///
    /// This is what lands in the `details` of an API error response, so an
    /// agent that builds a bad slug can tell which rule it broke and correct
    /// itself without a human reading the prose message.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Empty => "slug_empty",
            Self::TooLong => "slug_too_long",
            Self::SegmentTooLong { .. } => "slug_segment_too_long",
            Self::LeadingOrTrailingSlash => "slug_leading_or_trailing_slash",
            Self::EmptySegment => "slug_empty_segment",
            Self::RelativeSegment { .. } => "slug_relative_segment",
            Self::Backslash => "slug_backslash",
            Self::ForbiddenCharacter { .. } => "slug_forbidden_character",
            Self::SegmentPadding { .. } => "slug_segment_padding",
            Self::ReservedName { .. } => "slug_reserved_name",
        }
    }
}

impl Slug {
    /// Validate `raw` as a slug.
    pub fn parse(raw: &str) -> Result<Self, SlugError> {
        if raw.is_empty() {
            return Err(SlugError::Empty);
        }
        if raw.len() > MAX_SLUG_LEN {
            return Err(SlugError::TooLong);
        }
        // Checked before the character scan so the message names the separator
        // problem rather than reporting a generic forbidden character.
        if raw.contains('\\') {
            return Err(SlugError::Backslash);
        }
        if raw.starts_with('/') || raw.ends_with('/') {
            return Err(SlugError::LeadingOrTrailingSlash);
        }

        for character in raw.chars() {
            if character.is_control() {
                return Err(SlugError::ForbiddenCharacter { character });
            }
            if FORBIDDEN_CHARS.contains(&character) {
                return Err(SlugError::ForbiddenCharacter { character });
            }
        }

        for segment in raw.split('/') {
            Self::check_segment(segment)?;
        }

        Ok(Self(raw.to_owned()))
    }

    fn check_segment(segment: &str) -> Result<(), SlugError> {
        if segment.is_empty() {
            return Err(SlugError::EmptySegment);
        }
        if segment.len() > MAX_SEGMENT_LEN {
            return Err(SlugError::SegmentTooLong {
                segment: segment.to_owned(),
            });
        }
        if segment == "." || segment == ".." {
            return Err(SlugError::RelativeSegment {
                segment: segment.to_owned(),
            });
        }

        // Windows strips trailing dots and spaces, so `notes.` and `notes`
        // would be the same file there and different files on Linux. A leading
        // dot is rejected too: the wiki walker skips dot-entries, so such a
        // page would be written and then never indexed.
        let first = segment.chars().next().expect("segment is non-empty");
        let last = segment.chars().next_back().expect("segment is non-empty");
        if first == '.' || last == '.' || first.is_whitespace() || last.is_whitespace() {
            return Err(SlugError::SegmentPadding {
                segment: segment.to_owned(),
            });
        }

        // `CON.md` is still the console device, so the stem is what matters.
        let stem = segment.split('.').next().unwrap_or(segment);
        if RESERVED_STEMS
            .iter()
            .any(|reserved| reserved.eq_ignore_ascii_case(stem))
        {
            return Err(SlugError::ReservedName {
                segment: segment.to_owned(),
            });
        }

        Ok(())
    }

    /// Build the on-disk path for this slug under `root`.
    ///
    /// The final segment is pushed with `.md` already appended rather than
    /// going through `Path::set_extension`, which would rewrite the text after
    /// the last dot and turn the slug `notes/v1.2` into `notes/v1.md`.
    pub fn to_path(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        let mut segments = self.0.split('/').peekable();
        while let Some(segment) = segments.next() {
            if segments.peek().is_some() {
                path.push(segment);
            } else {
                path.push(format!("{segment}.md"));
            }
        }
        path
    }

    /// Recover a slug from a path relative to the wiki root.
    ///
    /// Returns `None` for anything that is not a `.md` file this module would
    /// have been willing to create, so the walker can skip stray files rather
    /// than indexing pages it could never serve.
    pub fn from_relative_path(path: &Path) -> Option<Self> {
        let mut segments = Vec::new();
        for component in path.components() {
            match component {
                Component::Normal(part) => segments.push(part.to_str()?),
                _ => return None,
            }
        }

        let (file_name, directories) = segments.split_last()?;
        let stem = file_name
            .strip_suffix(".md")
            .or_else(|| strip_suffix_ignore_ascii_case(file_name, ".md"))?;

        let mut raw = String::new();
        for directory in directories {
            raw.push_str(directory);
            raw.push('/');
        }
        raw.push_str(stem);

        Self::parse(&raw).ok()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The final path segment, used for the title fallback.
    pub fn basename(&self) -> &str {
        self.0.rsplit('/').next().unwrap_or(&self.0)
    }

    /// Everything before the final segment, if this slug is nested.
    pub fn parent(&self) -> Option<&str> {
        self.0.rsplit_once('/').map(|(parent, _)| parent)
    }
}

fn strip_suffix_ignore_ascii_case<'a>(value: &'a str, suffix: &str) -> Option<&'a str> {
    let split = value.len().checked_sub(suffix.len())?;
    let (head, tail) = value.split_at(split);
    tail.eq_ignore_ascii_case(suffix).then_some(head)
}

impl fmt::Display for Slug {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl AsRef<str> for Slug {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl FromStr for Slug {
    type Err = SlugError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::parse(value)
    }
}

impl From<Slug> for String {
    fn from(slug: Slug) -> Self {
        slug.0
    }
}

impl Serialize for Slug {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.0)
    }
}

/// Deserialising through [`Slug::parse`] means a slug read from JSON or from
/// frontmatter is validated at the boundary, not somewhere further in.
impl<'de> Deserialize<'de> for Slug {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = String::deserialize(deserializer)?;
        Self::parse(&raw).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn err(raw: &str) -> SlugError {
        Slug::parse(raw).expect_err("expected slug to be rejected")
    }

    #[test]
    fn accepts_ordinary_slugs() {
        for raw in [
            "index",
            "notes",
            "notes/rust/async",
            "a/b/c/d/e",
            "with-dashes_and_underscores",
            "with spaces inside",
            "unicode-rhizome-Ω-日本語",
            "digits123",
            "v1.2",            // dots are fine inside a segment
            "notes/v1.2/beta", // including in a directory
        ] {
            assert_eq!(Slug::parse(raw).map(String::from), Ok(raw.to_owned()));
        }
    }

    #[test]
    fn rejects_traversal() {
        assert_eq!(
            err(".."),
            SlugError::RelativeSegment {
                segment: "..".into()
            }
        );
        assert_eq!(
            err("../etc/passwd"),
            SlugError::RelativeSegment {
                segment: "..".into()
            }
        );
        assert_eq!(
            err("notes/../../etc/passwd"),
            SlugError::RelativeSegment {
                segment: "..".into()
            }
        );
        assert_eq!(
            err("notes/.."),
            SlugError::RelativeSegment {
                segment: "..".into()
            }
        );
        assert_eq!(
            err("."),
            SlugError::RelativeSegment {
                segment: ".".into()
            }
        );
        assert_eq!(
            err("notes/./async"),
            SlugError::RelativeSegment {
                segment: ".".into()
            }
        );
    }

    #[test]
    fn rejects_absolute_and_rooted_paths() {
        assert_eq!(err("/etc/passwd"), SlugError::LeadingOrTrailingSlash);
        assert_eq!(err("notes/"), SlugError::LeadingOrTrailingSlash);
        assert_eq!(err("/"), SlugError::LeadingOrTrailingSlash);
    }

    /// The cases Windows would accept and Linux would not, or vice versa.
    #[test]
    fn rejects_windows_specific_hazards() {
        // Drive prefixes and alternate data streams both ride in on `:`.
        assert_eq!(
            err("C:/Windows/System32"),
            SlugError::ForbiddenCharacter { character: ':' }
        );
        assert_eq!(
            err("notes:hidden"),
            SlugError::ForbiddenCharacter { character: ':' }
        );

        // Backslash is a separator on Windows and a legal filename character
        // on Linux; either way it must not appear.
        assert_eq!(err(r"..\..\windows\system32"), SlugError::Backslash);
        assert_eq!(err(r"notes\async"), SlugError::Backslash);

        // Windows silently strips these, collapsing two slugs into one file.
        assert_eq!(
            err("notes."),
            SlugError::SegmentPadding {
                segment: "notes.".into()
            }
        );
        assert_eq!(
            err("notes "),
            SlugError::SegmentPadding {
                segment: "notes ".into()
            }
        );
        assert_eq!(
            err("notes./async"),
            SlugError::SegmentPadding {
                segment: "notes.".into()
            }
        );

        // Device names, in any case, at any depth, with or without extension.
        assert_eq!(
            err("CON"),
            SlugError::ReservedName {
                segment: "CON".into()
            }
        );
        assert_eq!(
            err("con"),
            SlugError::ReservedName {
                segment: "con".into()
            }
        );
        assert_eq!(
            err("notes/NUL"),
            SlugError::ReservedName {
                segment: "NUL".into()
            }
        );
        assert_eq!(
            err("CON.backup"),
            SlugError::ReservedName {
                segment: "CON.backup".into()
            }
        );
        assert_eq!(
            err("com1/notes"),
            SlugError::ReservedName {
                segment: "com1".into()
            }
        );
        assert_eq!(
            err("LPT9"),
            SlugError::ReservedName {
                segment: "LPT9".into()
            }
        );

        // ...but only exact device stems.
        assert!(Slug::parse("console").is_ok());
        assert!(Slug::parse("nullable").is_ok());
        assert!(Slug::parse("com10").is_ok());
    }

    #[test]
    fn rejects_control_characters() {
        assert_eq!(
            err("notes\0async"),
            SlugError::ForbiddenCharacter { character: '\0' }
        );
        assert_eq!(
            err("notes\nasync"),
            SlugError::ForbiddenCharacter { character: '\n' }
        );
        assert_eq!(
            err("notes\tasync"),
            SlugError::ForbiddenCharacter { character: '\t' }
        );
        assert_eq!(
            err("notes\u{7f}async"),
            SlugError::ForbiddenCharacter {
                character: '\u{7f}'
            }
        );
    }

    #[test]
    fn rejects_dot_prefixed_segments() {
        // The walker skips dot-entries, so these would be write-only pages.
        assert_eq!(
            err(".hidden"),
            SlugError::SegmentPadding {
                segment: ".hidden".into()
            }
        );
        assert_eq!(
            err(".rhizowiki/index"),
            SlugError::SegmentPadding {
                segment: ".rhizowiki".into()
            }
        );
        assert_eq!(
            err("notes/.git"),
            SlugError::SegmentPadding {
                segment: ".git".into()
            }
        );
    }

    #[test]
    fn rejects_empty_and_oversized() {
        assert_eq!(err(""), SlugError::Empty);
        assert_eq!(err("notes//async"), SlugError::EmptySegment);
        assert_eq!(err(&"a".repeat(MAX_SLUG_LEN + 1)), SlugError::TooLong);

        let long = "b".repeat(MAX_SEGMENT_LEN + 1);
        assert_eq!(err(&long), SlugError::SegmentTooLong { segment: long });
    }

    #[test]
    fn builds_paths_under_the_root() {
        let root = Path::new("/wiki");
        assert_eq!(
            Slug::parse("index").unwrap().to_path(root),
            Path::new("/wiki/index.md")
        );
        assert_eq!(
            Slug::parse("notes/rust/async").unwrap().to_path(root),
            Path::new("/wiki/notes/rust/async.md")
        );
    }

    /// `Path::set_extension` would turn `notes/v1.2` into `notes/v1.md`,
    /// silently pointing two slugs at one file.
    #[test]
    fn dots_in_the_final_segment_survive() {
        let path = Slug::parse("notes/v1.2")
            .unwrap()
            .to_path(Path::new("/wiki"));
        assert_eq!(path, Path::new("/wiki/notes/v1.2.md"));
    }

    #[test]
    fn round_trips_through_a_relative_path() {
        for raw in ["index", "notes/rust/async", "v1.2"] {
            let slug = Slug::parse(raw).unwrap();
            let path = slug.to_path(Path::new(""));
            assert_eq!(Slug::from_relative_path(&path).as_ref(), Some(&slug));
        }
    }

    #[test]
    fn ignores_paths_that_are_not_pages() {
        for path in ["notes/async.txt", "notes/async", "README", ".git/config"] {
            assert_eq!(Slug::from_relative_path(Path::new(path)), None);
        }
        // Case-insensitive filesystems will hand us either spelling.
        assert!(Slug::from_relative_path(Path::new("notes/Async.MD")).is_some());
    }

    #[test]
    fn exposes_basename_and_parent() {
        let nested = Slug::parse("notes/rust/async").unwrap();
        assert_eq!(nested.basename(), "async");
        assert_eq!(nested.parent(), Some("notes/rust"));

        let flat = Slug::parse("index").unwrap();
        assert_eq!(flat.basename(), "index");
        assert_eq!(flat.parent(), None);
    }

    #[test]
    fn deserialising_validates() {
        assert!(serde_json::from_str::<Slug>(r#""notes/async""#).is_ok());
        assert!(serde_json::from_str::<Slug>(r#""../etc/passwd""#).is_err());
    }
}
