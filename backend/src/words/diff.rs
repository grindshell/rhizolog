//! How many words were added, and how many removed, between two bodies.
//!
//! A pure function of two strings, which is the only reason its edge cases are
//! testable at all.

use std::collections::HashMap;

use crate::markdown;

/// What changed between two versions of a page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Churn {
    pub added: u64,
    pub removed: u64,
}

impl Churn {
    /// The net change, which is arithmetic over the two rather than a stored
    /// value. It is the number this feature exists to stop being the only one.
    pub fn delta(self) -> i64 {
        self.added as i64 - self.removed as i64
    }

    pub fn is_nothing(self) -> bool {
        self.added == 0 && self.removed == 0
    }
}

/// Words added and removed, going from `before` to `after`.
///
/// ## It counts words, not arrangement
///
/// The comparison is between two **multisets** of words: `added` is every word
/// in the new text that the old text did not have a copy of, and `removed` is
/// the reverse. Moving a paragraph is therefore not writing it again, and
/// swapping two sentences is not sixty words of churn.
///
/// The cost of that is stated rather than hidden: **a pure reordering reports
/// nothing**. A day spent restructuring a chapter without writing a new word is
/// a day the log says nothing happened on. The alternative is a sequence diff,
/// which would call a moved paragraph both added and removed, and whose longest
/// common subsequence over prose is padded out by `the` and `and` anyway. This
/// is the version somebody can check by hand, which is the same reason
/// `tfidf/v1` has no stemming.
///
/// It also makes the log's own check exact. `added - removed` is always the
/// change in the page's word count, so a line can be verified against the page
/// rather than believed:
///
/// ```text
/// added - removed == count_words(after) - count_words(before)
/// ```
///
/// ## What counts as a word
///
/// Whatever [`markdown::count_words`] counts, which is the extracted text of the
/// page: no code, no raw HTML blocks, no link targets, no frontmatter. Wrapping
/// a paragraph in a block quote, or reflowing it, therefore reports nothing,
/// which is the whole reason this runs over the extracted text rather than over
/// the markdown.
///
/// Case is significant, because changing `the` to `The` is an edit. It is one
/// word added and one removed, and the page's total is unmoved, which is exactly
/// what happened.
pub fn churn(before: &str, after: &str) -> Churn {
    let mut counts: HashMap<String, i64> = HashMap::new();

    for word in markdown::words(before) {
        *counts.entry(core(&word)).or_default() -= 1;
    }
    for word in markdown::words(after) {
        *counts.entry(core(&word)).or_default() += 1;
    }

    let mut churn = Churn::default();

    for count in counts.into_values() {
        if count > 0 {
            churn.added += count as u64;
        } else {
            churn.removed += count.unsigned_abs();
        }
    }

    churn
}

/// A word with the punctuation stuffed against it taken off.
///
/// The counter splits on whitespace, so extending a sentence turns `late.` into
/// `late` and would otherwise report one word removed and two added where one
/// was written. Trimming both ends is what makes moving a full stop cost
/// nothing.
///
/// It cannot empty a word: a word is a run holding at least one alphanumeric
/// character, so there is always something between the punctuation. And it
/// cannot break the arithmetic below, because it changes what counts as the
/// **same** word and never how many there are.
///
/// Interior punctuation stays, so `it's` and `rust-lang` are each one word and
/// not two. Case stays too: changing `the` to `The` is an edit, and it comes out
/// as one word each way with the page's total unmoved, which is exactly what
/// happened.
fn core(word: &str) -> String {
    word.trim_matches(|character: char| !character.is_alphanumeric())
        .to_owned()
}

/// A net change split by its sign, for when the previous body is gone.
///
/// The one place a net figure is allowed, and it is written into the log as
/// [`super::Kind::Net`] so that nobody reads it as a churn. It happens when
/// `index.db` was deleted and pages changed before the next start: the log knows
/// what the page's total was and the file says what it is now, and the
/// difference is all there is.
pub fn net(before: u64, after: u64) -> Churn {
    Churn {
        added: after.saturating_sub(before),
        removed: before.saturating_sub(after),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property that makes `total` a check rather than a claim.
    fn adds_up(before: &str, after: &str) {
        let churn = churn(before, after);
        let expected = markdown::count_words(after) as i64 - markdown::count_words(before) as i64;

        assert_eq!(
            churn.delta(),
            expected,
            "{churn:?} does not account for the change in the page's count"
        );
    }

    /// The example the whole feature exists for: a rewrite is not "minus one
    /// hundred".
    #[test]
    fn a_rewrite_reports_both_halves_and_not_their_difference() {
        let before = "The ferry was late again, and nobody on the quay seemed surprised.\n";
        let after = "Nothing about the delay astonished anyone waiting there.\n";

        let churn = churn(before, after);

        assert!(churn.added > 0 && churn.removed > 0, "got {churn:?}");
        assert!(
            churn.delta() < 0,
            "the page got shorter, so the net is negative: {churn:?}"
        );
        adds_up(before, after);
    }

    #[test]
    fn an_append_is_added_and_nothing_else() {
        let churn = churn("One two three.\n", "One two three. Four five.\n");

        assert_eq!(
            churn,
            Churn {
                added: 2,
                removed: 0
            }
        );
    }

    #[test]
    fn a_deletion_is_removed_and_nothing_else() {
        let churn = churn("One two three. Four five.\n", "One two three.\n");

        assert_eq!(
            churn,
            Churn {
                added: 0,
                removed: 2
            }
        );
    }

    #[test]
    fn saving_a_page_nobody_touched_is_no_churn_at_all() {
        let body = "# A heading\n\nAnd a paragraph under it.\n";

        assert!(churn(body, body).is_nothing());
        assert!(churn("", "").is_nothing());
    }

    /// A page that only ever gained words has an empty other half.
    #[test]
    fn a_new_page_is_all_addition() {
        assert_eq!(churn("", "Three whole words.\n").added, 3);
        assert_eq!(churn("Three whole words.\n", "").removed, 3);
    }

    /// The documented blind spot, pinned so it is chosen rather than accidental.
    #[test]
    fn a_pure_reordering_reports_nothing() {
        let before = "The ferry was late. Nobody was surprised.\n";
        let after = "Nobody was surprised. The ferry was late.\n";

        assert!(churn(before, after).is_nothing());
    }

    /// Formatting is not writing. This is why the diff runs over the extracted
    /// text rather than over the markdown.
    #[test]
    fn reflowing_and_quoting_are_not_words() {
        assert!(churn("One two three four five.\n", "One two\nthree four five.\n").is_nothing());
        assert!(churn("One two three.\n", "> One two three.\n").is_nothing());
        assert!(
            churn("One two three.\n", "One *two* three.\n").is_nothing(),
            "emphasis leaves the words alone"
        );
    }

    /// Code is not prose, so pasting a function into a page is not a thousand
    /// words written.
    #[test]
    fn a_code_block_is_not_words() {
        let before = "Prose.\n";
        let after = "Prose.\n\n```rust\nfn main() { println!(\"lots of words here\"); }\n```\n";

        assert!(churn(before, after).is_nothing());
        adds_up(before, after);
    }

    /// Extending a sentence moves a full stop onto a different word. Counting
    /// that as a word removed and two added would over-report every sentence
    /// anybody ever finished.
    #[test]
    fn moving_a_full_stop_is_not_a_word() {
        assert_eq!(
            churn("One two three four.\n", "One two three four five.\n"),
            Churn {
                added: 1,
                removed: 0
            }
        );
        adds_up("One two three four.\n", "One two three four five.\n");

        // And the punctuation *inside* a word is part of it.
        assert!(churn("it's a rust-lang thing\n", "it's a rust-lang thing\n").is_nothing());
        assert_eq!(
            churn("the rust-lang crate\n", "the rustlang crate\n"),
            Churn {
                added: 1,
                removed: 1
            }
        );
    }

    #[test]
    fn changing_a_capital_is_one_word_each_way() {
        let churn = churn("the ferry\n", "The ferry\n");

        assert_eq!(
            churn,
            Churn {
                added: 1,
                removed: 1
            }
        );
        assert_eq!(churn.delta(), 0);
    }

    /// A word repeated more times than before is added that many times, not once.
    #[test]
    fn repeats_are_counted_with_their_multiplicity() {
        assert_eq!(churn("late\n", "late late late\n").added, 2);
        assert_eq!(churn("late late late\n", "late\n").removed, 2);
    }

    #[test]
    fn a_net_is_a_change_split_by_its_sign() {
        assert_eq!(
            net(100, 140),
            Churn {
                added: 40,
                removed: 0
            }
        );
        assert_eq!(
            net(140, 100),
            Churn {
                added: 0,
                removed: 40
            }
        );
        assert!(net(100, 100).is_nothing());
    }
}
