//! Turning a body into the units the rules talk about: tokens and sentences.
//!
//! Extraction itself lives in [`crate::markdown`], which is the module that owns
//! walking comrak's AST and which already decides what counts as prose. This one
//! holds the two things built on top of it, and every offset it produces is a
//! byte offset into the body it was given. That is the property the whole
//! feature rests on: the editor is a textarea over the source, and a finding you
//! cannot find is not a finding.

use std::ops::Range;

use crate::markdown::{self, Extracted};

/// One token: a run of alphanumeric characters, and where it is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Token {
    /// The token lowercased, which is what every rule but `consistent` compares.
    pub folded: String,
    /// Byte offset of the first character, in the body.
    pub start: usize,
    /// Byte offset one past the last.
    pub end: usize,
}

impl Token {
    pub fn span(&self) -> Range<usize> {
        self.start..self.end
    }
}

/// One sentence, and how many tokens are in it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sentence {
    pub start: usize,
    pub end: usize,
    pub words: usize,
}

/// A body and everything the rules read off it, computed once.
///
/// Five rules over one document would otherwise re-parse and re-tokenize it five
/// times, and worse, could disagree about the answer.
#[derive(Debug, Clone)]
pub struct Text<'a> {
    /// The body as written. Quotes are cut from here, so what a finding shows is
    /// what the writer typed rather than a reconstruction of it.
    pub source: &'a str,
    pub extracted: Extracted,
    pub tokens: Vec<Token>,
    pub sentences: Vec<Sentence>,
}

impl<'a> Text<'a> {
    pub fn of(source: &'a str) -> Self {
        let extracted = markdown::extract(source);
        let tokens = tokens(&extracted.text);
        let sentences = sentences(&extracted, &tokens);

        Self {
            source,
            extracted,
            tokens,
            sentences,
        }
    }

    /// The body between two offsets, for a finding to quote.
    ///
    /// Cut from the source rather than from the blanked text, so a quote spanning
    /// a link reads as the link and not as a hole. Empty rather than a panic if
    /// the span does not land on character boundaries, which it always should.
    pub fn quote(&self, span: Range<usize>) -> String {
        self.source.get(span).unwrap_or_default().to_owned()
    }
}

/// Split text into tokens: runs of alphanumeric characters, in source order.
///
/// The tokenizer `knowledge-base/idea-inbox.md` defines for `tfidf/v1`, with one
/// deliberate difference. That one lowercases the whole
/// string *before* splitting, so that a character whose lowercase form includes
/// a combining mark splits the way the folded text reads. Doing it that way here
/// is impossible: Rust's lowercase conversion can change a string's length, and
/// every byte offset after the first such character would be wrong. So the split
/// happens over the text as written and each token is folded on its own. The two
/// differ only for characters whose lowercase form is not alphanumeric
/// throughout, and a wrong offset is a worse failure than a wrong fold.
///
/// No stemming and no stop-word list, for the reason that page gives: every
/// signal a rule shows has to appear literally in text the writer wrote. `delve`
/// does not match `delved`, and a rule that wants both lists both.
pub fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    let mut open: Option<usize> = None;

    for (index, character) in text.char_indices() {
        if character.is_alphanumeric() {
            open.get_or_insert(index);
        } else if let Some(start) = open.take() {
            tokens.push(token(text, start, index));
        }
    }

    if let Some(start) = open {
        tokens.push(token(text, start, text.len()));
    }

    tokens
}

fn token(text: &str, start: usize, end: usize) -> Token {
    Token {
        folded: text[start..end].to_lowercase(),
        start,
        end,
    }
}

/// The sentences of a body, in source order.
///
/// Only paragraphs are read, which is [`Extracted::paragraphs`]'s whole reason
/// for existing, and a sentence never crosses from one paragraph into the next.
/// A run of them may, though, because five one-sentence paragraphs of the same
/// length is the same tell as five sentences of the same length in one.
fn sentences(extracted: &Extracted, tokens: &[Token]) -> Vec<Sentence> {
    let mut sentences = Vec::new();

    for paragraph in &extracted.paragraphs {
        for span in split(&extracted.text, paragraph.clone()) {
            let words = count_in(tokens, &span);

            // A stretch of punctuation with no word in it is not a sentence, and
            // letting one through would put a zero into every mean it lands in.
            if words > 0 {
                sentences.push(Sentence {
                    words,
                    start: span.start,
                    end: span.end,
                });
            }
        }
    }

    sentences
}

/// Split one paragraph into sentences.
///
/// **Sentence splitting is a hazard and this is the honest version of it**: a
/// `.`, `?` or `!`, then whitespace, then an uppercase letter. `Dr. Kaltenbrunner
/// arrived` splits wrongly and always will without a list of abbreviations, which
/// is a thing this project is not going to carry. The one rule built on it,
/// `uniformity`, is about runs rather than exact lengths, so a wrong split moves
/// a number by a few words instead of inventing a finding. Pinned by a test so
/// the behaviour is chosen rather than accidental.
fn split(text: &str, paragraph: Range<usize>) -> Vec<Range<usize>> {
    let Some(slice) = text.get(paragraph.clone()) else {
        return Vec::new();
    };

    let characters: Vec<(usize, char)> = slice.char_indices().collect();
    let mut spans = Vec::new();
    let mut begin = 0;
    let mut index = 0;

    while index < characters.len() {
        let (at, character) = characters[index];

        if matches!(character, '.' | '?' | '!') {
            let mut next = index + 1;
            while next < characters.len() && characters[next].1.is_whitespace() {
                next += 1;
            }

            if next > index + 1
                && let Some(&(start, upper)) = characters.get(next)
                && upper.is_uppercase()
            {
                push(
                    slice,
                    begin..at + character.len_utf8(),
                    paragraph.start,
                    &mut spans,
                );
                begin = start;
                index = next;
                continue;
            }
        }

        index += 1;
    }

    push(slice, begin..slice.len(), paragraph.start, &mut spans);
    spans
}

/// Trim a candidate sentence to what it actually says, and drop it if that is
/// nothing. The blanked text is mostly whitespace, so this is where a paragraph
/// made entirely of markup stops being a sentence.
fn push(slice: &str, span: Range<usize>, offset: usize, spans: &mut Vec<Range<usize>>) {
    let Some(text) = slice.get(span.clone()) else {
        return;
    };

    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }

    let start = span.start + (text.len() - text.trim_start().len());
    spans.push(offset + start..offset + start + trimmed.len());
}

/// Whether a token sits somewhere a capital letter is forced anyway.
///
/// The question `consistent` has to ask before it calls a capitalised word a
/// name. `If` and `It` are not two spellings of one name, they are two ordinary
/// words that happen to start sentences, and on real prose that one confusion
/// produced most of the rule's findings.
///
/// True at the very start of the text, after a `.`, `?` or `!`, and after a
/// blank line, which is where a heading or a new paragraph begins. It is
/// deliberately conservative: every case it gets wrong costs a candidate rather
/// than inventing one, which is the direction to be wrong in for the one rule
/// nobody wrote.
pub fn opens_a_sentence(text: &str, at: usize) -> bool {
    let Some(before) = text.get(..at) else {
        return true;
    };

    let mut newlines = 0;

    for character in before.chars().rev() {
        if character == '\n' {
            newlines += 1;
            if newlines >= 2 {
                return true;
            }
        } else if !character.is_whitespace() {
            return matches!(character, '.' | '?' | '!');
        }
    }

    true
}

/// How many tokens begin inside a span.
fn count_in(tokens: &[Token], span: &Range<usize>) -> usize {
    tokens
        .iter()
        .filter(|token| token.start >= span.start && token.start < span.end)
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folded(text: &str) -> Vec<String> {
        Text::of(text)
            .tokens
            .into_iter()
            .map(|token| token.folded)
            .collect()
    }

    fn said(text: &str) -> Vec<String> {
        let parsed = Text::of(text);
        parsed
            .sentences
            .iter()
            .map(|sentence| parsed.quote(sentence.start..sentence.end))
            .collect()
    }

    /// The property everything else rests on: an offset out of here indexes the
    /// body that went in.
    #[test]
    fn every_token_indexes_the_source_it_came_from() {
        let source = "A caf\u{e9} and a [link](notes/a.md) here.\n";
        let parsed = Text::of(source);

        for token in &parsed.tokens {
            assert_eq!(
                source[token.span()].to_lowercase(),
                token.folded,
                "token {token:?} does not name its own bytes"
            );
        }
    }

    #[test]
    fn code_and_link_targets_are_not_tokens() {
        assert_eq!(folded("Call `std::mem::swap` now.\n"), ["call", "now"]);
        assert_eq!(folded("```rust\nfn main() {}\n```\n\nProse.\n"), ["prose"]);
        assert_eq!(
            folded("See [the notes](notes/rust/async.md).\n"),
            ["see", "the", "notes"],
            "a link says its text; where it points is not part of what it says"
        );
        assert_eq!(folded("Look: ![a red bicycle](bike.png)\n"), ["look"]);
    }

    /// A bare wikilink displays its slug, so the slug is what is read. The
    /// separators are not letters, so it is three tokens rather than one word.
    #[test]
    fn a_bare_wikilink_is_read_as_its_slug() {
        assert_eq!(
            folded("See [[notes/rust/async]].\n"),
            ["see", "notes", "rust", "async"]
        );
        assert_eq!(
            folded("See [[notes/a|the notes]].\n"),
            ["see", "the", "notes"]
        );
    }

    /// The documented disagreement with the word counter. `un*believable*` is one
    /// word there and two tokens here, because the `*` between them is blanked
    /// and a token is a run of alphanumerics.
    #[test]
    fn markup_inside_a_word_splits_it() {
        assert_eq!(markdown::count_words("un*believable*\n"), 1);
        assert_eq!(folded("un*believable*\n"), ["un", "believable"]);
    }

    #[test]
    fn tokens_are_folded_but_their_bytes_are_not() {
        let parsed = Text::of("The Ferry was LATE.\n");
        assert_eq!(
            parsed
                .tokens
                .iter()
                .map(|token| token.folded.as_str())
                .collect::<Vec<_>>(),
            ["the", "ferry", "was", "late"]
        );
        assert_eq!(parsed.source[parsed.tokens[1].span()].to_owned(), "Ferry");
    }

    #[test]
    fn splits_on_a_stop_then_whitespace_then_a_capital() {
        assert_eq!(
            said("One thing happened. Then another did.\n"),
            ["One thing happened.", "Then another did."]
        );
        assert_eq!(
            said("Did it? It did! Twice.\n"),
            ["Did it?", "It did!", "Twice."]
        );
    }

    /// A soft wrap is not a sentence boundary, which matters because this
    /// repository's own prose is wrapped at eighty columns.
    #[test]
    fn a_soft_wrap_does_not_end_a_sentence() {
        assert_eq!(
            said("One sentence that runs\nacross two lines here.\n"),
            ["One sentence that runs\nacross two lines here."]
        );
    }

    /// The known failure, pinned so it is chosen rather than accidental.
    #[test]
    fn an_abbreviation_splits_wrongly_and_that_is_the_documented_answer() {
        assert_eq!(
            said("Dr. Kaltenbrunner arrived late.\n"),
            ["Dr.", "Kaltenbrunner arrived late."]
        );
        // A decimal point does not, because a digit is not uppercase.
        assert_eq!(said("It cost 3.50 in total.\n"), ["It cost 3.50 in total."]);
    }

    /// Rhythm is a property of a paragraph. A list is uniform by construction and
    /// a heading is not a sentence, so neither is measured.
    #[test]
    fn only_paragraphs_have_sentences() {
        assert_eq!(
            said("# A heading\n\nA paragraph.\n\n- one item\n- two item\n"),
            ["A paragraph."]
        );
        assert!(said("| a | b |\n|---|---|\n| c | d |\n").is_empty());
        assert!(said("```\nfn main() {}\n```\n").is_empty());
    }

    /// A block quote is prose somebody chose to include, and a footnote is prose
    /// somebody wrote.
    #[test]
    fn quoted_and_footnoted_prose_are_sentences_too() {
        assert_eq!(said("> A quoted claim.\n"), ["A quoted claim."]);
        assert_eq!(
            said("A claim.[^1]\n\n[^1]: The note.\n"),
            ["A claim.", "The note."],
            "and in the order they were written, not the order comrak stores them"
        );
    }

    /// Two paragraphs are never one sentence, whatever punctuation the first
    /// ended with.
    #[test]
    fn a_sentence_never_crosses_a_paragraph() {
        assert_eq!(
            said("No full stop here\n\nAnd a second paragraph\n"),
            ["No full stop here", "And a second paragraph"]
        );
    }

    #[test]
    fn a_sentence_counts_its_own_words() {
        let parsed = Text::of("One two three. Four five.\n");
        assert_eq!(
            parsed
                .sentences
                .iter()
                .map(|sentence| sentence.words)
                .collect::<Vec<_>>(),
            [3, 2]
        );
    }

    /// Where a capital letter is forced, and where it says something.
    #[test]
    fn a_forced_capital_is_told_from_a_chosen_one() {
        let source = "The ferry left. Kaltenbrunner met it.\n\nA new paragraph.\n";
        let parsed = Text::of(source);
        let text = &parsed.extracted.text;

        let at = |word: &str| text.find(word).expect("the word is in the text");

        assert!(opens_a_sentence(text, at("The")), "the start of the body");
        assert!(
            opens_a_sentence(text, at("Kaltenbrunner")),
            "after a full stop"
        );
        assert!(
            opens_a_sentence(text, at("A new")),
            "after a blank line, which is where a paragraph or a heading begins"
        );
        assert!(
            !opens_a_sentence(text, at("ferry")),
            "in the middle of a sentence, where a capital would have meant something"
        );
    }

    #[test]
    fn nothing_at_all_is_no_tokens_and_no_sentences() {
        let parsed = Text::of("");
        assert!(parsed.tokens.is_empty());
        assert!(parsed.sentences.is_empty());
    }
}
