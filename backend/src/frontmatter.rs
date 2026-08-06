//! Splitting a YAML frontmatter block from the markdown that follows it.
//!
//! Two kinds of file in a wiki have this shape: a [page](crate::page) and a
//! [time entry](crate::times). Both are hand-editable, so both have to survive
//! the same set of small horrors — a BOM written by Notepad, CRLF line endings,
//! a fence that is never closed, a horizontal rule that only looks like one.
//! Those answers live here rather than being written twice, because the second
//! copy is where they quietly diverge.

use serde::de::DeserializeOwned;
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
}
