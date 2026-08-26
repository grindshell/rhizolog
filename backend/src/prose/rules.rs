//! The five rule kinds, each a pure function of a body and its own options.
//!
//! Nothing here reaches the disk, the index, or the network. Every one of them
//! takes a [`Text`] and returns findings, which is what makes their edge cases
//! testable, and follows `ideas/{analysis,lifecycle}.rs`.
//!
//! Code and raw HTML are excluded from every rule, always, and from the AST
//! rather than from configuration. A page documenting a syntax should not be
//! flagged for containing it. That exclusion is [`crate::markdown::extract`]'s
//! doing, so no rule here has to remember it.

use std::collections::{BTreeMap, HashSet};

use serde_json::json;

use super::text::{Sentence, Text, Token};
use super::{Finding, Options, Rule};

/// Everything one rule has to say about one body.
pub fn findings(rule: &Rule, text: &Text) -> Vec<Finding> {
    match &rule.options {
        Options::Forbid { literals } => forbid(rule, text, literals),
        Options::Phrase { phrases } => phrase(rule, text, phrases),
        Options::Echo { within, ignore } => echo(rule, text, *within, ignore),
        Options::Uniformity { run, spread } => uniformity(rule, text, *run, *spread),
        Options::Consistent {
            distance,
            length,
            allow,
        } => consistent(rule, text, *distance, *length, allow),
    }
}

/// A literal, wherever it appears in the prose.
///
/// Over the extracted text rather than over tokens, which is what lets a rule
/// name a single character: this repository's own em dash rule has no word in it
/// at all. Matching folds ASCII case and nothing else, because folding the rest
/// would mean lowercasing the haystack, and Rust's lowercase conversion can
/// change a string's length: every offset after the first such character would
/// then be wrong, and a finding you cannot find is not a finding.
fn forbid(rule: &Rule, text: &Text, literals: &[String]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for literal in literals {
        for span in occurrences(&text.extracted.text, literal) {
            findings.push(Finding {
                rule: rule.id.clone(),
                severity: rule.severity,
                start: span.0,
                end: span.1,
                quote: text.quote(span.0..span.1),
                message: rule
                    .message
                    .clone()
                    .unwrap_or_else(|| format!("forbidden: {literal}")),
                receipt: json!({ "literal": literal }),
            });
        }
    }

    findings
}

/// Every non-overlapping occurrence of `needle`, as byte ranges.
///
/// A match always lands on character boundaries even though the search is over
/// bytes, because UTF-8 is self-synchronising: a needle's first byte is either
/// ASCII or a lead byte, and neither can occur in the middle of a character.
fn occurrences(haystack: &str, needle: &str) -> Vec<(usize, usize)> {
    let (hay, need) = (haystack.as_bytes(), needle.as_bytes());

    if need.is_empty() || need.len() > hay.len() {
        return Vec::new();
    }

    let mut found = Vec::new();
    let mut at = 0;

    while at + need.len() <= hay.len() {
        if hay[at..at + need.len()].eq_ignore_ascii_case(need) {
            found.push((at, at + need.len()));
            at += need.len();
        } else {
            at += 1;
        }
    }

    found
}

/// A sequence of tokens, so `delve into` does not fire inside a word.
///
/// The span runs from the first token's start to the last one's end, which means
/// it covers whatever punctuation sat between them. That is deliberate: `delve,
/// into` is the phrase with a comma in it, not two coincidences.
fn phrase(rule: &Rule, text: &Text, phrases: &[String]) -> Vec<Finding> {
    let mut findings = Vec::new();

    for spelling in phrases {
        let wanted: Vec<String> = super::text::tokens(spelling)
            .into_iter()
            .map(|token| token.folded)
            .collect();

        if wanted.is_empty() {
            continue;
        }

        let mut at = 0;
        while at + wanted.len() <= text.tokens.len() {
            let run = &text.tokens[at..at + wanted.len()];

            if run
                .iter()
                .zip(&wanted)
                .all(|(token, word)| &token.folded == word)
            {
                let (start, end) = (run[0].start, run[run.len() - 1].end);

                findings.push(Finding {
                    rule: rule.id.clone(),
                    severity: rule.severity,
                    start,
                    end,
                    quote: text.quote(start..end),
                    message: rule
                        .message
                        .clone()
                        .unwrap_or_else(|| format!("phrase: {spelling}")),
                    receipt: json!({ "phrase": spelling, "words": wanted }),
                });

                at += wanted.len();
            } else {
                at += 1;
            }
        }
    }

    findings
}

/// The same word twice, close together.
///
/// Reports the **second** occurrence and spans both, so the finding shows the
/// stretch of prose that has the repeat in it rather than one word out of
/// context. A run of three produces two findings, first to second and second to
/// third, because each is a repeat of what came before it.
///
/// `ignore` is not a stop-word list smuggled back in: it is authored, it is per
/// rule, and it defaults to empty.
fn echo(rule: &Rule, text: &Text, within: usize, ignore: &[String]) -> Vec<Finding> {
    let ignored: HashSet<&str> = ignore.iter().map(String::as_str).collect();
    let mut findings = Vec::new();

    for (index, token) in text.tokens.iter().enumerate() {
        if ignored.contains(token.folded.as_str()) {
            continue;
        }

        let earliest = index.saturating_sub(within);

        // Backwards, so the nearest previous occurrence is the one reported.
        // Taking the furthest instead would make a run of three into one wide
        // finding and hide the tighter repeat inside it.
        let Some(before) = (earliest..index)
            .rev()
            .find(|&back| text.tokens[back].folded == token.folded)
        else {
            continue;
        };

        let (first, distance) = (&text.tokens[before], index - before);

        findings.push(Finding {
            rule: rule.id.clone(),
            severity: rule.severity,
            start: first.start,
            end: token.end,
            quote: text.quote(first.start..token.end),
            message: rule
                .message
                .clone()
                .unwrap_or_else(|| format!("{} repeated within {distance} words", token.folded)),
            receipt: json!({
                "token": token.folded,
                "first": first.start,
                "second": token.start,
                "distance": distance,
                "within": within,
            }),
        });
    }

    findings
}

/// A run of sentences all about the same length.
///
/// The tell that a passage was written by something with no ear: not a bad
/// sentence anywhere, and every one of them the same size. Only the longest run
/// is reported, never the shorter ones inside it, which is what the skip past
/// `end` below is for.
fn uniformity(rule: &Rule, text: &Text, run: usize, spread: f64) -> Vec<Finding> {
    let sentences = &text.sentences;
    let mut findings = Vec::new();
    let mut at = 0;

    while at < sentences.len() {
        let mut end = at + 1;
        while end < sentences.len() && even(&sentences[at..end + 1], spread) {
            end += 1;
        }

        if end - at < run {
            at += 1;
            continue;
        }

        let window = &sentences[at..end];
        let lengths: Vec<usize> = window.iter().map(|sentence| sentence.words).collect();
        let mean = mean_of(window);
        let (start, stop) = (window[0].start, window[window.len() - 1].end);

        findings.push(Finding {
            rule: rule.id.clone(),
            severity: rule.severity,
            start,
            end: stop,
            quote: text.quote(start..stop),
            message: rule.message.clone().unwrap_or_else(|| {
                format!(
                    "{} sentences within {spread} words of {mean:.1}",
                    window.len()
                )
            }),
            receipt: json!({
                "lengths": lengths,
                "mean": mean,
                "run": run,
                "spread": spread,
            }),
        });

        at = end;
    }

    findings
}

fn mean_of(window: &[Sentence]) -> f64 {
    window
        .iter()
        .map(|sentence| sentence.words as f64)
        .sum::<f64>()
        / window.len() as f64
}

/// Whether every sentence in a window sits within `spread` of the window's mean.
///
/// Checked over the whole window each time it grows rather than incrementally,
/// because the mean moves as it does: a sentence that was inside the band can
/// fall out of it when a longer one joins.
fn even(window: &[Sentence], spread: f64) -> bool {
    let mean = mean_of(window);

    window
        .iter()
        .all(|sentence| (sentence.words as f64 - mean).abs() <= spread)
}

/// One name, spelled two ways.
///
/// The only rule nobody wrote, and the only one with real false positives, which
/// is why it is also the only one with an `allow` list. Both spellings have to
/// appear at least twice, so a single typo is not enough to accuse an established
/// name of being the mistake, and both have to start with an uppercase character,
/// which is what keeps it to names rather than to every near-miss in the
/// language.
///
/// It reads the token **as written**. It is the one rule about spelling, and
/// folding case first would make `Ferry` and `ferry` the same word, which is
/// exactly the distinction it exists to see.
///
/// Two filters beyond that, and both were argued for by running this over real
/// documents rather than by anybody's taste:
///
/// - **Capitalised where a capital was not already forced.** `If` and `It` are
///   not two spellings of one name. See [`super::text::opens_a_sentence`].
/// - **At least `length` characters.** What survived the first filter was
///   initialisms and labels: `L0` beside `L1`, `UTF` beside `UTC`.
fn consistent(
    rule: &Rule,
    text: &Text,
    distance: usize,
    length: usize,
    allow: &[String],
) -> Vec<Finding> {
    let allowed: HashSet<&str> = allow.iter().map(String::as_str).collect();
    let mut spellings: BTreeMap<&str, Vec<&Token>> = BTreeMap::new();

    for token in &text.tokens {
        let Some(written) = text.source.get(token.span()) else {
            continue;
        };
        spellings.entry(written).or_default().push(token);
    }

    let candidates: Vec<(&str, &Vec<&Token>)> = spellings
        .iter()
        .filter(|(written, seen)| {
            seen.len() >= 2
                && !allowed.contains(*written)
                && written.chars().count() >= length
                && written.chars().next().is_some_and(char::is_uppercase)
                // Capitalised somewhere a capital was not forced. A name is a
                // name in the middle of a sentence too; `If` and `It` are not.
                && seen.iter().any(|token| {
                    !super::text::opens_a_sentence(&text.extracted.text, token.start)
                })
        })
        .map(|(written, seen)| (*written, seen))
        .collect();

    let mut findings = Vec::new();

    for (index, (left, left_seen)) in candidates.iter().enumerate() {
        for (right, right_seen) in &candidates[index + 1..] {
            let apart = edits(left, right, distance);
            if apart > distance {
                continue;
            }

            // The rarer spelling is the one being reported, on the reading that
            // the commoner one is what the writer settled on. A tie goes to
            // whichever appeared first, for the same reason, and then to the
            // alphabet so that two runs never disagree about which is which.
            let (odd, odd_seen, usual, usual_seen) =
                if settled(left, left_seen) > settled(right, right_seen) {
                    (*right, right_seen, *left, left_seen)
                } else {
                    (*left, left_seen, *right, right_seen)
                };

            for token in odd_seen.iter() {
                findings.push(Finding {
                    rule: rule.id.clone(),
                    severity: rule.severity,
                    start: token.start,
                    end: token.end,
                    quote: text.quote(token.span()),
                    message: rule.message.clone().unwrap_or_else(|| {
                        format!(
                            "{odd} ({}) beside {usual} ({})",
                            odd_seen.len(),
                            usual_seen.len()
                        )
                    }),
                    receipt: json!({
                        "spelling": odd,
                        "count": odd_seen.len(),
                        "other": usual,
                        "other_count": usual_seen.len(),
                        "distance": apart,
                        "maximum": distance,
                    }),
                });
            }
        }
    }

    findings
}

/// How settled a spelling looks: more of them, earlier, wins.
///
/// The greater of two is the one taken as what the writer meant. Reversing the
/// last two is what turns "smaller is earlier" into "greater is settled", so one
/// comparison covers all three.
fn settled<'a>(written: &'a str, seen: &[&Token]) -> (usize, std::cmp::Reverse<(usize, &'a str)>) {
    (seen.len(), std::cmp::Reverse((seen[0].start, written)))
}

/// Damerau-Levenshtein distance, counting a transposition as one edit.
///
/// Optimal string alignment rather than the unrestricted variant, which is the
/// version that treats `ab` to `ba` as one edit and is what a typo actually is.
/// Returns `ceiling + 1` for anything further apart than that, so the caller's
/// comparison is the only thing that decides.
fn edits(left: &str, right: &str, ceiling: usize) -> usize {
    let left: Vec<char> = left.chars().collect();
    let right: Vec<char> = right.chars().collect();
    let (rows, columns) = (left.len(), right.len());

    if rows.abs_diff(columns) > ceiling {
        return ceiling + 1;
    }

    let mut grid = vec![vec![0usize; columns + 1]; rows + 1];
    for (row, line) in grid.iter_mut().enumerate() {
        line[0] = row;
    }
    for (column, cell) in grid[0].iter_mut().enumerate() {
        *cell = column;
    }

    for row in 1..=rows {
        for column in 1..=columns {
            let cost = usize::from(left[row - 1] != right[column - 1]);

            grid[row][column] = (grid[row - 1][column] + 1)
                .min(grid[row][column - 1] + 1)
                .min(grid[row - 1][column - 1] + cost);

            if row > 1
                && column > 1
                && left[row - 1] == right[column - 2]
                && left[row - 2] == right[column - 1]
            {
                grid[row][column] = grid[row][column].min(grid[row - 2][column - 2] + 1);
            }
        }
    }

    grid[rows][columns]
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::prose::{Analysis, Severity, analyze, parse};

    fn found(rules: &str, body: &str) -> Vec<Finding> {
        let ruleset = parse(rules).expect("the rules parse");
        let Analysis { findings, .. } = analyze(body, &ruleset);
        findings
    }

    /// Every span has to index the body it was found in, or the finding is one
    /// nobody can act on.
    fn quotes_are_honest(body: &str, findings: &[Finding]) {
        for finding in findings {
            assert_eq!(
                &body[finding.start..finding.end],
                finding.quote,
                "{finding:?} does not quote its own span"
            );
        }
    }

    // ------------------------------------------------------------------ forbid

    /// This repository's own rule, and the reason the parser has to take an
    /// escape: the file that configures it may not contain the character.
    #[test]
    fn forbid_finds_a_character_this_file_cannot_hold() {
        let rules = "[[rule]]\nid = \"no-em-dash\"\nkind = \"forbid\"\nseverity = \"error\"\n\
                     literals = [\"\\u2014\"]\nmessage = \"em dash\"\n";
        let body = "A clause \u{2014} and another.\n";
        let findings = found(rules, body);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].severity, Severity::Error);
        assert_eq!(findings[0].message, "em dash");
        assert_eq!(findings[0].quote, "\u{2014}");
        assert_eq!(findings[0].receipt["literal"], "\u{2014}");
        quotes_are_honest(body, &findings);
    }

    /// The reason every rule goes through the AST. A page documenting a syntax
    /// must not be flagged for containing it.
    #[test]
    fn forbid_does_not_see_code_or_link_targets() {
        let rules = "[[rule]]\nid = \"no-dash\"\nkind = \"forbid\"\nliterals = [\"--\"]\n";

        assert!(found(rules, "Run `cargo test --all` now.\n").is_empty());
        assert!(found(rules, "```\ncargo test --all\n```\n").is_empty());
        assert!(found(rules, "See [notes](a--b.md).\n").is_empty());
        assert_eq!(found(rules, "A -- dash in prose.\n").len(), 1);
    }

    #[test]
    fn forbid_folds_ascii_case_and_does_not_overlap_itself() {
        let rules = "[[rule]]\nid = \"no-aa\"\nkind = \"forbid\"\nliterals = [\"Aa\"]\n";
        assert_eq!(found(rules, "aaaa here\n").len(), 2);
    }

    // ------------------------------------------------------------------ phrase

    #[test]
    fn a_phrase_is_a_run_of_tokens_and_not_a_substring() {
        let rules = "[[rule]]\nid = \"tells\"\nkind = \"phrase\"\nphrases = [\"delve into\"]\n";

        let body = "We delve into it, and delve, into it again.\n";
        let findings = found(rules, body);

        assert_eq!(
            findings.len(),
            2,
            "punctuation between tokens is not a defence"
        );
        assert_eq!(findings[0].quote, "delve into");
        assert_eq!(findings[1].quote, "delve, into");
        quotes_are_honest(body, &findings);

        assert!(
            found(rules, "Nobody delved into anything.\n").is_empty(),
            "no stemming: a rule that wants delved lists delved"
        );
    }

    // -------------------------------------------------------------------- echo

    #[test]
    fn echo_reports_the_second_occurrence_and_spans_both() {
        let rules = "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 6\n";
        let body = "The ferry was late, and late is what it was.\n";
        let findings = found(rules, body);

        let late: Vec<&Finding> = findings
            .iter()
            .filter(|finding| finding.receipt["token"] == "late")
            .collect();

        assert_eq!(late.len(), 1);
        assert_eq!(late[0].quote, "late, and late");
        assert_eq!(late[0].receipt["distance"], 2);
        assert_eq!(late[0].message, "late repeated within 2 words");
        quotes_are_honest(body, &findings);
    }

    #[test]
    fn a_run_of_three_is_two_findings_and_ignore_silences_a_word() {
        let rules = "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 4\n";
        let body = "one and one and one\n";

        let ones: Vec<Finding> = found(rules, body)
            .into_iter()
            .filter(|finding| finding.receipt["token"] == "one")
            .collect();
        assert_eq!(ones.len(), 2);
        assert!(ones.iter().all(|finding| finding.receipt["distance"] == 2));

        let quiet = "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 4\n\
                     ignore = [\"and\", \"one\"]\n";
        assert!(found(quiet, body).is_empty());
    }

    #[test]
    fn echo_does_not_reach_further_than_it_was_told_to() {
        let rules = "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 2\n";
        assert!(found(rules, "one two three four one\n").is_empty());
    }

    // -------------------------------------------------------------- uniformity

    #[test]
    fn uniformity_reports_the_longest_run_and_nothing_inside_it() {
        let rules = "[[rule]]\nid = \"uniformity\"\nkind = \"uniformity\"\nrun = 4\nspread = 1\n";
        let body = "One two three four. Two three four five. Three four five six. \
                    Four five six seven. Five six seven eight.\n";

        let findings = found(rules, body);
        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].receipt["lengths"], json!([4, 4, 4, 4, 4]));
        assert_eq!(findings[0].receipt["mean"], 4.0);
        quotes_are_honest(body, &findings);
    }

    #[test]
    fn uniformity_leaves_prose_with_a_rhythm_alone() {
        let rules = "[[rule]]\nid = \"uniformity\"\nkind = \"uniformity\"\nrun = 3\nspread = 1\n";
        let body = "Short. A rather longer sentence follows it here now. Then this. \
                    And then one that goes on for a good deal longer than any of them.\n";

        assert!(found(rules, body).is_empty());
    }

    /// A list is uniform by construction, so measuring one would be a rule
    /// nobody leaves switched on.
    #[test]
    fn uniformity_does_not_measure_a_bulleted_list() {
        let rules = "[[rule]]\nid = \"uniformity\"\nkind = \"uniformity\"\nrun = 3\nspread = 1\n";
        let body = "- one two three\n- two three four\n- three four five\n- four five six\n";

        assert!(found(rules, body).is_empty());
    }

    // -------------------------------------------------------------- consistent

    /// Both names appear in the middle of a sentence, which is what makes them
    /// names rather than words that happen to start one.
    const TWO_SPELLINGS: &str = "The ferry brought Kaltenbrunner. Nobody met Kaltenbrunner. \
                                 Later, Kaltenbruner left. Nobody saw Kaltenbruner.\n";

    #[test]
    fn consistent_reports_every_occurrence_of_the_rarer_spelling() {
        let rules = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\ndistance = 1\n";
        let body = TWO_SPELLINGS;

        let findings = found(rules, body);
        assert_eq!(findings.len(), 2);
        assert!(
            findings
                .iter()
                .all(|finding| finding.quote == "Kaltenbruner")
        );
        assert_eq!(findings[0].receipt["count"], 2);
        assert_eq!(findings[0].receipt["other"], "Kaltenbrunner");
        assert_eq!(findings[0].receipt["other_count"], 2);
        assert_eq!(findings[0].receipt["distance"], 1);
        quotes_are_honest(body, &findings);
    }

    #[test]
    fn consistent_is_the_one_rule_that_reads_case_as_written() {
        let rules = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\n";
        // `ferry` never starts with a capital, so it is not a name and is not
        // compared against `Ferry`.
        let body = "The Ferry left. The Ferry sank. A ferry is a boat. Another ferry too.\n";

        assert!(found(rules, body).is_empty());
    }

    #[test]
    fn a_single_appearance_is_a_typo_rather_than_a_spelling() {
        let rules = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\n";
        let body = "The ferry brought Kaltenbrunner. Nobody met Kaltenbrunner. \
                    Later, Kaltenbruner left.\n";

        assert!(found(rules, body).is_empty());
    }

    #[test]
    fn allow_is_how_the_one_rule_nobody_wrote_gets_told_it_is_wrong() {
        let rules = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\n\
                     allow = [\"Kaltenbruner\"]\n";

        assert!(found(rules, TWO_SPELLINGS).is_empty());
    }

    /// The false positive that dominated the rule's output on real prose, and
    /// the reason a candidate has to be capitalised somewhere a capital was not
    /// already forced. `If` and `It` are not two spellings of one name.
    #[test]
    fn a_word_that_only_ever_starts_a_sentence_is_not_a_name() {
        // `length = 1` so that this test is about where the words sit and not
        // about how short they are, which is the other filter.
        let rules = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\nlength = 1\n";
        let body = "If the ferry runs. It will be late. If nobody comes. It sails anyway.\n";

        assert!(found(rules, body).is_empty());

        // The same two words, once each in the middle of a sentence, are back to
        // being candidates: the filter is about where they sit, not what they are.
        let named = "If the If runs. It will be It. If nobody comes. It sails anyway.\n";
        assert!(!found(rules, named).is_empty());
    }

    /// What was left after the sentence filter, on real documents: `L0` beside
    /// `L1`, `UTF` beside `UTC`. Deliberately different labels, one edit apart.
    #[test]
    fn an_initialism_is_not_a_misspelling_of_another_initialism() {
        let rules = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\n";
        let body = "Phase L0 ships, then L1 ships. After L0 comes L1 in the plan.\n";

        assert!(found(rules, body).is_empty());

        let shorter = "[[rule]]\nid = \"names\"\nkind = \"consistent\"\nlength = 2\n";
        assert!(
            !found(shorter, body).is_empty(),
            "a wiki full of two-letter names says so"
        );
    }

    #[test]
    fn a_transposition_is_one_edit() {
        assert_eq!(edits("Kaltenbrunner", "Kaltenbrunenr", 4), 1);
        assert_eq!(edits("abc", "abc", 4), 0);
        assert_eq!(edits("abc", "xyz", 4), 3);
        assert_eq!(
            edits("a", "abcdefg", 2),
            3,
            "further apart than the ceiling"
        );
    }

    // ----------------------------------------------------------------- overall

    /// Two rules may fire on overlapping spans and both are reported, because
    /// suppressing one would mean ranking rules against each other.
    #[test]
    fn overlapping_findings_are_both_reported_in_a_fixed_order() {
        let rules = "[[rule]]\nid = \"b-phrase\"\nkind = \"phrase\"\nphrases = [\"very late\"]\n\n\
                     [[rule]]\nid = \"a-forbid\"\nkind = \"forbid\"\nliterals = [\"very\"]\n";
        let body = "It was very late.\n";
        let findings = found(rules, body);

        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].rule, "a-forbid", "ties break on the rule id");
        assert_eq!(findings[1].rule, "b-phrase");
        assert_eq!(findings[0].start, findings[1].start);
    }

    /// Spans are bytes, so a multi-byte character before a finding has to move
    /// it by more than one.
    #[test]
    fn a_span_survives_a_multi_byte_character_ahead_of_it() {
        let rules = "[[rule]]\nid = \"tells\"\nkind = \"phrase\"\nphrases = [\"a testament\"]\n";
        let body = "The caf\u{e9} was a testament to something.\n";
        let findings = found(rules, body);

        assert_eq!(findings.len(), 1);
        assert_eq!(findings[0].quote, "a testament");
        quotes_are_honest(body, &findings);
    }

    #[test]
    fn no_rules_means_no_findings_and_no_work() {
        let ruleset = parse("").expect("an empty file is an empty ruleset");
        assert!(ruleset.is_empty());
        assert!(analyze("Anything at all.\n", &ruleset).findings.is_empty());
    }
}
