//! Splitting a YAML frontmatter block from the markdown that follows it.
//!
//! Two kinds of file in a wiki have this shape: a [page](crate::page) and a
//! [time entry](crate::times). Both are hand-editable, so both have to survive
//! the same set of small horrors — a BOM written by Notepad, CRLF line endings,
//! a fence that is never closed, a horizontal rule that only looks like one.
//! Those answers live here rather than being written twice, because the second
//! copy is where they quietly diverge.

use chrono::{DateTime, NaiveDate, NaiveTime, Utc};
use serde::de::{DeserializeOwned, Error as _};
use serde::{Deserialize, Deserializer};
use thiserror::Error;

pub const DELIMITER: &str = "---";

#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("frontmatter opens with `---` but is never closed")]
    Unterminated,

    #[error("frontmatter is not valid YAML: {0}")]
    InvalidYaml(#[from] serde_yaml_ng::Error),
}

/// Drop a leading UTF-8 byte order mark.
///
/// `read_to_string` keeps the BOM — U+FEFF is a perfectly valid character — so
/// a file saved by Notepad, by PowerShell's `Set-Content -Encoding utf8`, or by
/// an editor set to "UTF-8 with BOM" begins with three bytes that are invisible
/// to a person and fatal to frontmatter detection: the text no longer starts
/// with `---`, the whole block is read as body, and every field in it is
/// silently lost.
///
/// Development is on Windows, where writing a BOM is the *default* for several
/// common tools, so this is the likeliest way for a hand-authored file to be
/// misparsed — and it fails quietly, which is what makes it worth handling here
/// rather than expecting an author to notice.
///
/// The BOM is not written back: a file that round-trips through the API comes
/// out normalised without one.
pub fn strip_bom(text: &str) -> &str {
    text.strip_prefix('\u{feff}').unwrap_or(text)
}

/// Split leading frontmatter from the body.
///
/// Returns `Ok(None)` when the text does not open with a frontmatter fence at
/// all, which is the ordinary case for a hand-written file.
pub fn split(text: &str) -> Result<Option<(&str, &str)>, FrontmatterError> {
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

    Err(FrontmatterError::Unterminated)
}

/// Parse a frontmatter block into its fields.
///
/// An empty block is legal and means "no fields", but YAML parses the empty
/// document as null rather than as an empty mapping, so it is handled here.
pub fn parse<T: DeserializeOwned + Default>(yaml: &str) -> Result<T, FrontmatterError> {
    if yaml.trim().is_empty() {
        return Ok(T::default());
    }
    Ok(serde_yaml_ng::from_str(yaml)?)
}

/// Read an optional timestamp that may have been written as a bare date.
///
/// Rhizolog writes `2026-08-19T10:00:00Z` and chrono's own deserialiser wants
/// exactly that. A person writing the field by hand writes `2026-08-19`, and
/// refusing it does not cost them the field — it costs them the **page**, since
/// a frontmatter block that will not parse makes the whole file malformed and
/// drops it out of every listing, title, tags and all. That is a heavy price for
/// a date somebody wrote the ordinary way.
///
/// A bare date means **midnight UTC**. There is no time in it to lose, so the
/// only question is which convention to fill in, and UTC is the one the rest of
/// this codebase already speaks.
///
/// A *naive datetime* — `2026-08-19T10:00:00`, with no zone — is still refused,
/// and the difference is the point. A bare date carries no time, so supplying
/// one invents nothing; a wall-clock time with no zone carries a real time whose
/// meaning depends on where it was written, and reading it as UTC would silently
/// move it by up to fourteen hours. Being told is better.
pub fn timestamp<'de, D>(deserializer: D) -> Result<Option<DateTime<Utc>>, D::Error>
where
    D: Deserializer<'de>,
{
    // Through `String` rather than through chrono, because YAML resolves a plain
    // scalar against the type it is going into: both spellings arrive here as
    // text, and neither has been interpreted yet.
    let Some(raw) = Option::<String>::deserialize(deserializer)? else {
        return Ok(None);
    };

    parse_timestamp(&raw)
        .map(Some)
        .ok_or_else(|| D::Error::custom(format!(
            "expected a timestamp like 2026-08-19T10:00:00Z or a date like 2026-08-19, found {raw:?}"
        )))
}

fn parse_timestamp(raw: &str) -> Option<DateTime<Utc>> {
    let text = raw.trim();

    if let Ok(stamp) = DateTime::parse_from_rfc3339(text) {
        return Some(stamp.with_timezone(&Utc));
    }

    NaiveDate::parse_from_str(text, "%Y-%m-%d")
        .ok()
        .map(|date| date.and_time(NaiveTime::MIN).and_utc())
}

/// Put a fenced YAML block back in front of a body.
pub fn compose(yaml: &str, body: &str) -> String {
    let mut out = String::with_capacity(yaml.len() + body.len() + 16);
    out.push_str(DELIMITER);
    out.push('\n');
    out.push_str(yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(DELIMITER);
    out.push('\n');
    out.push_str(body);
    out
}

fn strip_one_line_ending(text: &str) -> Option<&str> {
    text.strip_prefix("\r\n")
        .or_else(|| text.strip_prefix('\n'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_a_fenced_block_from_the_body() {
        let (yaml, body) = split("---\na: 1\n---\nBody.\n").unwrap().unwrap();
        assert_eq!(yaml, "a: 1\n");
        assert_eq!(body, "Body.\n");
    }

    /// Windows editors write CRLF, and this all runs on Windows.
    #[test]
    fn handles_crlf_line_endings() {
        let (yaml, body) = split("---\r\na: 1\r\n---\r\nBody.\r\n").unwrap().unwrap();
        assert_eq!(yaml, "a: 1\r\n");
        assert_eq!(body, "Body.\r\n");
    }

    #[test]
    fn a_body_that_merely_contains_a_rule_is_not_frontmatter() {
        assert!(split("Text.\n\n---\n\nMore.\n").unwrap().is_none());
    }

    #[test]
    fn rejects_an_unterminated_block() {
        assert!(matches!(
            split("---\na: 1\n\nBody.\n"),
            Err(FrontmatterError::Unterminated)
        ));
    }

    #[test]
    fn composing_is_the_inverse_of_splitting() {
        let text = "---\na: 1\n---\nBody.\n";
        let (yaml, body) = split(text).unwrap().unwrap();
        assert_eq!(compose(yaml, body), text);
    }

    #[test]
    fn a_bom_is_stripped_but_only_at_the_front() {
        assert_eq!(strip_bom("\u{feff}---\n"), "---\n");
        assert_eq!(strip_bom("a\u{feff}b"), "a\u{feff}b");
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    /// A stand-in for the frontmatter types that carry a timestamp, so this
    /// module's tests do not have to reach for a page or an account.
    #[derive(Debug, Default, serde::Deserialize)]
    struct Held {
        #[serde(default, deserialize_with = "timestamp")]
        created: Option<DateTime<Utc>>,
    }

    #[test]
    fn a_timestamp_is_read_in_full_or_as_a_bare_date() {
        assert_eq!(
            parse_timestamp("2026-08-19T10:00:00Z"),
            Some(at("2026-08-19T10:00:00Z"))
        );
        // An offset is honoured and normalised, as it always was.
        assert_eq!(
            parse_timestamp("2026-08-19T12:00:00+02:00"),
            Some(at("2026-08-19T10:00:00Z"))
        );
        // The date somebody actually types. Midnight UTC invents no time, since
        // there was none there to begin with.
        assert_eq!(
            parse_timestamp("2026-08-19"),
            Some(at("2026-08-19T00:00:00Z"))
        );
        // Quoted is the same string, and surrounding space is not a difference
        // worth failing a whole page over.
        assert_eq!(
            parse_timestamp("  2026-08-19  "),
            Some(at("2026-08-19T00:00:00Z"))
        );
    }

    /// A wall-clock time with no zone is a real time whose meaning depends on
    /// where it was written. Reading it as UTC would move it silently by up to
    /// fourteen hours, which is worse than saying so.
    #[test]
    fn a_time_without_a_zone_is_still_refused() {
        assert_eq!(parse_timestamp("2026-08-19T10:00:00"), None);
        assert_eq!(parse_timestamp("2026-08-19 10:00:00"), None);
    }

    #[test]
    fn nonsense_is_refused_rather_than_guessed_at() {
        for raw in [
            "yes",
            "",
            "2026",
            "2026-08",
            "19-08-2026",
            "2026-13-01",
            "2026-08-19T",
            "tomorrow",
        ] {
            assert_eq!(parse_timestamp(raw), None, "{raw:?} was accepted");
        }
    }

    /// The message a reader gets has to name what would have worked. This is a
    /// field somebody is editing by hand, by definition.
    #[test]
    fn the_refusal_says_what_would_have_worked() {
        let error = parse::<Held>("created: tomorrow\n").expect_err("refused");
        let message = error.to_string();

        assert!(message.contains("2026-08-19T10:00:00Z"), "{message}");
        assert!(message.contains("2026-08-19"), "{message}");
        assert!(message.contains("tomorrow"), "{message}");
    }

    #[test]
    fn an_absent_or_null_timestamp_is_simply_absent() {
        assert_eq!(parse::<Held>("title: x\n").unwrap().created, None);
        assert_eq!(parse::<Held>("created: null\n").unwrap().created, None);
        assert_eq!(parse::<Held>("created:\n").unwrap().created, None);
    }
}
