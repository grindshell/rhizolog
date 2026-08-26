//! Dividing one page into two, folding one back into another, and repairing the
//! `contents:` lists that named it.
//!
//! [`crate::compile`] assembles a spine and [`crate::api::spine`] edits one. This
//! module is the arithmetic in between: it knows nothing about HTTP, the store or
//! who is asking, and everything here is one string in and one string out.
//!
//! Two rules shape it, and both are argued in
//! `knowledge-base/split-and-merge.md`:
//!
//! - **Nothing is written and nothing is unwritten.** A split moves a boundary
//!   and a merge removes one. The words on either side are the words that were
//!   there, which is why [`crate::words::Kind::Split`] and
//!   [`crate::words::Kind::Merged`] are markers rather than churn.
//! - **An entry nothing could resolve is still something somebody wrote.** A
//!   repair matches a contents entry as a string, so a gap, a repeat and a typo
//!   all keep their positions across one.

/// Why an offset could not divide a body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetError {
    /// Past the end of the body.
    Outside,
    /// Inside a character.
    ///
    /// Reachable for a caller that counted bytes wrong, and only ever for a page
    /// with something outside ASCII in it, which is exactly the page where the
    /// mistake is hardest to see.
    NotABoundary,
    /// One of the two halves would have nothing in it.
    ///
    /// Checked **after** the cut rather than by looking at the offset, which is
    /// the only way to be right about it: the blank lines at the seam belong to
    /// neither half, so an offset well inside a body can still leave one side
    /// empty. The last newline of a page is the case that matters, because
    /// clicking at the end of the last line of text is where a caret lands.
    ///
    /// A split that produced a blank page would put one in the spine, and one
    /// that emptied the page it was cutting would be a rename with a step
    /// missing. Both have endpoints of their own.
    OneSided,
}

impl OffsetError {
    /// What to tell the caller, phrased as the rule that was broken.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Outside => "an offset has to fall inside the body, and this is past the end",
            Self::NotABoundary => "an offset has to fall between characters, not inside one",
            Self::OneSided => "a split has to leave text on both sides, and this leaves one blank",
        }
    }
}

/// Cut a body in two at a byte offset.
///
/// Bytes rather than characters or lines, because that is the unit
/// [`crate::prose`] spans already use: a finding quotes a span of the page
/// source, and an offset saying where a scene ends is the same kind of number
/// about the same text. A client that can reveal a finding in a textarea can
/// already produce one of these.
///
/// The blank lines at the cut belong to neither half and are dropped. A tail that
/// began with the newline ending the previous paragraph would open on a blank
/// line, and a head that ended mid-air would run into whatever is appended to it
/// later. Only newlines come off the front, never indentation, so a tail that
/// starts inside an indented block keeps its shape.
///
/// **Both halves are checked after the cut, not before it.** Dropping those blank
/// lines is what makes an offset well inside a body still able to leave one side
/// with nothing in it, so an offset is only known to be a place to split once the
/// splitting is done. `at` at the last newline of a page is the case worth
/// naming: it is where a caret lands when somebody clicks at the end of the text,
/// and unchecked it produces a blank page and puts it in the spine.
pub fn divide(body: &str, at: usize) -> Result<(String, String), OffsetError> {
    if at > body.len() {
        return Err(OffsetError::Outside);
    }
    if !body.is_char_boundary(at) {
        return Err(OffsetError::NotABoundary);
    }

    let (head, tail) = body.split_at(at);
    let (head, tail) = (ending(head), ending(tail.trim_start_matches(['\n', '\r'])));

    if head.is_empty() || tail.is_empty() {
        return Err(OffsetError::OneSided);
    }

    Ok((head, tail))
}

/// Put two bodies together, in that order.
///
/// A blank line between them, which is what markdown needs to keep the last
/// paragraph of one from joining the first paragraph of the other. An empty half
/// contributes nothing rather than a blank line at the top or the bottom.
///
/// This is not quite the inverse of [`divide`]: a body cut inside a paragraph and
/// joined again gains a paragraph break where it had none. The words are the same
/// either way, which is what the word log records.
pub fn join(head: &str, tail: &str) -> String {
    let head = head.trim_end();
    // The same rule the tail of a [`divide`] gets, for the same reason: newlines
    // at the seam belong to nobody, and indentation belongs to the block it is
    // holding open.
    let tail = tail.trim_start_matches(['\n', '\r']).trim_end();

    match (head.is_empty(), tail.is_empty()) {
        (true, true) => String::new(),
        (true, false) => ending(tail),
        (false, true) => ending(head),
        (false, false) => format!("{head}\n\n{tail}\n"),
    }
}

/// A body as it belongs in a file: one trailing newline, no trailing blanks.
fn ending(text: &str) -> String {
    let text = text.trim_end();
    if text.is_empty() {
        String::new()
    } else {
        format!("{text}\n")
    }
}

/// Put `entry` into `list` after every occurrence of `after`.
///
/// `None` when the list is left as it was, so a caller can tell a repair from a
/// page it does not have to write.
///
/// **A list that already names `entry` is left alone.** Splitting into a chapter
/// somebody outlined and has not written is filling a gap, and adding a second
/// entry for it would turn the gap into a `duplicate` for them to clean up. Where
/// they put it is where it stays: the position is theirs, and this has no better
/// one to offer.
pub fn insert_after(list: &[String], after: &str, entry: &str) -> Option<Vec<String>> {
    if list.iter().any(|item| item == entry) || !list.iter().any(|item| item == after) {
        return None;
    }

    let mut next = Vec::with_capacity(list.len() + 1);
    for item in list {
        next.push(item.clone());
        if item == after {
            next.push(entry.to_owned());
        }
    }

    Some(next)
}

/// Take every occurrence of `entry` out of `list`.
///
/// `None` when there was none, on the same terms as [`insert_after`]. Every
/// occurrence rather than the first, because a page listed twice is gone twice
/// once it is gone.
pub fn without(list: &[String], entry: &str) -> Option<Vec<String>> {
    if !list.iter().any(|item| item == entry) {
        return None;
    }

    Some(
        list.iter()
            .filter(|item| item.as_str() != entry)
            .cloned()
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::markdown::count_words;

    const CHAPTER: &str = "# The Ferry\n\nHe missed the crossing.\n\n# The Return\n\nHe came \
                           back in the dark.\n";

    fn list(entries: &[&str]) -> Vec<String> {
        entries.iter().map(|entry| (*entry).to_owned()).collect()
    }

    #[test]
    fn a_body_divides_at_a_heading() {
        let at = CHAPTER.find("# The Return").expect("second heading");
        let (head, tail) = divide(CHAPTER, at).expect("divide");

        assert_eq!(head, "# The Ferry\n\nHe missed the crossing.\n");
        assert_eq!(tail, "# The Return\n\nHe came back in the dark.\n");
    }

    /// The point of the whole feature: the words are the words, before and
    /// after. It is what makes the log's two markers honest.
    #[test]
    fn dividing_at_a_line_neither_writes_nor_unwrites_anything() {
        let at = CHAPTER.find("# The Return").expect("second heading");
        let (head, tail) = divide(CHAPTER, at).expect("divide");

        assert_eq!(
            count_words(&head) + count_words(&tail),
            count_words(CHAPTER)
        );
    }

    #[test]
    fn joining_two_halves_puts_the_chapter_back() {
        let at = CHAPTER.find("# The Return").expect("second heading");
        let (head, tail) = divide(CHAPTER, at).expect("divide");

        assert_eq!(join(&head, &tail), CHAPTER);
    }

    /// The blank line at the cut belongs to neither half.
    #[test]
    fn a_tail_never_opens_on_a_blank_line() {
        let (head, tail) = divide("One.\n\n\n\nTwo.\n", 5).expect("divide");

        assert_eq!(head, "One.\n");
        assert_eq!(tail, "Two.\n");
    }

    /// Only newlines come off the front. A tail that begins inside a fenced or
    /// indented block keeps the indentation that makes it one.
    #[test]
    fn a_tail_keeps_the_indentation_it_starts_with() {
        let (_, tail) = divide("Prose.\n\n    indented code\n", 7).expect("divide");
        assert_eq!(tail, "    indented code\n");
    }

    #[test]
    fn an_offset_with_nothing_on_one_side_of_it_is_refused() {
        assert_eq!(divide(CHAPTER, 0), Err(OffsetError::OneSided));
        assert_eq!(divide(CHAPTER, CHAPTER.len()), Err(OffsetError::OneSided));
        assert_eq!(
            divide(CHAPTER, CHAPTER.len() + 1),
            Err(OffsetError::Outside)
        );
        // A page with nothing in it has no offset that would work, and says so
        // rather than producing two empty pages.
        assert_eq!(divide("", 0), Err(OffsetError::OneSided));
    }

    /// The case a bounds check misses and a caret finds. Clicking at the end of
    /// the last line of text lands on the final newline, which is inside the body
    /// by every measure and still leaves the second half with nothing in it.
    /// Unchecked, it writes a blank page and puts it in the spine.
    #[test]
    fn the_last_newline_of_a_page_is_not_a_place_to_split() {
        assert_eq!(
            divide(CHAPTER, CHAPTER.len() - 1),
            Err(OffsetError::OneSided)
        );
        assert_eq!(divide("One two three.\n", 14), Err(OffsetError::OneSided));
        // Trailing blank lines are the same case, however many of them there are.
        assert_eq!(divide("One.\n\n\n\n", 5), Err(OffsetError::OneSided));
    }

    /// The other end of it, which empties the page being cut rather than the one
    /// being made. The blank lines at the seam belong to neither half, so this is
    /// an offset a bounds check calls perfectly good.
    #[test]
    fn an_offset_that_would_empty_the_page_being_split_is_refused() {
        assert_eq!(divide("\n\n\nText here.\n", 2), Err(OffsetError::OneSided));
        assert_eq!(divide("   \nText here.\n", 4), Err(OffsetError::OneSided));
    }

    /// A caller counting UTF-16 units and sending them as bytes lands here, on
    /// the one kind of page where the mistake is invisible.
    #[test]
    fn an_offset_inside_a_character_is_refused() {
        let body = "café\n\nau lait\n";
        assert_eq!(divide(body, 4), Err(OffsetError::NotABoundary));
        assert!(divide(body, 5).is_ok());
    }

    #[test]
    fn joining_tolerates_a_half_with_nothing_in_it() {
        assert_eq!(join("", "Two.\n"), "Two.\n");
        assert_eq!(join("One.\n", ""), "One.\n");
        assert_eq!(join("", ""), "");
        assert_eq!(join("One.", "Two."), "One.\n\nTwo.\n");
    }

    #[test]
    fn an_entry_goes_in_after_the_one_it_was_split_from() {
        assert_eq!(
            insert_after(&list(&["a", "b", "c"]), "b", "b2"),
            Some(list(&["a", "b", "b2", "c"]))
        );
    }

    /// The doubled case `page_parts` is keyed for. Both positions are that page,
    /// and the second half belongs after each of them.
    #[test]
    fn an_entry_listed_twice_gains_a_neighbour_in_both_places() {
        assert_eq!(
            insert_after(&list(&["a", "b", "a"]), "a", "a2"),
            Some(list(&["a", "a2", "b", "a", "a2"]))
        );
    }

    /// Splitting into a chapter somebody already outlined fills the gap. A
    /// second entry for it would be a `duplicate` they have to clean up.
    #[test]
    fn a_list_that_already_names_the_new_page_is_left_alone() {
        assert_eq!(insert_after(&list(&["a", "b", "a2"]), "a", "a2"), None);
    }

    #[test]
    fn a_list_that_never_named_the_page_is_left_alone() {
        assert_eq!(insert_after(&list(&["a", "b"]), "c", "c2"), None);
    }

    #[test]
    fn a_merged_page_leaves_every_list_that_named_it() {
        assert_eq!(
            without(&list(&["a", "b", "a"]), "a"),
            Some(list(&["b"])),
            "a page listed twice is gone from both positions"
        );
        assert_eq!(without(&list(&["a", "b"]), "c"), None);
    }

    /// An entry a compile could make nothing of is still something somebody
    /// wrote, and it keeps its position across a repair.
    #[test]
    fn a_gap_a_repeat_and_a_typo_all_survive_a_repair() {
        let spine = list(&["written", "unwritten", "../etc/passwd", "written"]);

        assert_eq!(
            insert_after(&spine, "written", "second-half"),
            Some(list(&[
                "written",
                "second-half",
                "unwritten",
                "../etc/passwd",
                "written",
                "second-half",
            ]))
        );
        assert_eq!(
            without(&spine, "written"),
            Some(list(&["unwritten", "../etc/passwd"]))
        );
    }
}
