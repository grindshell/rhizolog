//! `prose/v1`: holding prose to rules the author wrote down.
//!
//! Not a grammar checker and not a style guide. Somebody drafting long-form work
//! with an assistant is not fighting typos, they are fighting **drift**, and
//! drift is invisible from the inside because you read the prose as it arrives.
//! So every rule here is one the author wrote, in one file, and every finding
//! comes back with the arithmetic that produced it.
//!
//! The contract is the one `tfidf/v1` set in `knowledge-base/idea-inbox.md` and
//! it is worth restating because it is the whole value of the feature:
//!
//! - **Local, deterministic, no network, and no model.** Not even inside a rule.
//!   A second machine opinion about your voice is the problem this exists to
//!   answer, not a way of answering it.
//! - **Versioned.** Changing any rule's arithmetic, the tokenizer, or the
//!   sentence splitter is `prose/v2` and a note on
//!   `knowledge-base/long-form.md`.
//! - **Every finding quotes the text it fired on and carries a receipt**, which
//!   is the numbers the rule actually compared. Without it, "late repeated within
//!   12 words" is a sentence asking to be believed rather than an arithmetic
//!   anybody can check.
//!
//! There is no dismissal store, deliberately. Idea Inbox needs rejections
//! because a machine proposes something about your data and can be wrong about
//! it. A prose finding is your own rule firing on your own text: if it fires
//! where it should not, the rule is wrong, and the fix is to edit the rules file.
//! One place, no event store, nothing to fold.

pub mod rules;
pub mod text;

use std::collections::BTreeSet;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use thiserror::Error;
use utoipa::ToSchema;

use crate::store::INTERNAL_DIR;

/// The name this analyzer answers to, returned with every finding.
pub const ANALYZER: &str = "prose/v1";

/// Where the rules live, under [`crate::store::INTERNAL_DIR`].
///
/// Authored configuration: the only copy, worth committing, and not secret, so
/// it sits with `times/` and `ideas/` rather than with `users/`. It is read on
/// every request rather than cached, which is what makes tuning a rule a matter
/// of saving the file and asking again.
pub const RULES_FILE: &str = "prose.toml";

/// How many rules one file may hold.
pub const MAX_RULES: usize = 200;

/// How many findings one response may carry.
///
/// A `forbid` rule naming a single common letter would otherwise return one
/// finding per occurrence across a whole book. Unlike a compile, a short answer
/// here is safe to give because it says it is short: see [`Analysis::truncated`].
pub const MAX_FINDINGS: usize = 1_000;

pub const DEFAULT_WITHIN: usize = 40;
pub const MAX_WITHIN: usize = 200;
pub const DEFAULT_RUN: usize = 5;
pub const MAX_RUN: usize = 100;
pub const DEFAULT_SPREAD: f64 = 3.0;
pub const MAX_SPREAD: f64 = 1000.0;
pub const DEFAULT_DISTANCE: usize = 1;
/// Beyond about four edits, two capitalised words are simply two words.
pub const MAX_DISTANCE: usize = 4;
/// How short a spelling may be and still be taken for a name.
///
/// Four, which is what running `consistent` over this project's own documents
/// argued for: every false positive left after the sentence-initial filter was
/// an initialism or a label, `L0` beside `L1`, `UTF` beside `UTC`. A name is
/// longer than that, and a wiki with a three-letter one lowers the number.
pub const DEFAULT_LENGTH: usize = 4;
pub const MAX_LENGTH: usize = 64;

/// How loudly a rule speaks.
///
/// It carries no behaviour: nothing here blocks a save, and a linter that could
/// would be a linter with an opinion about when you are allowed to write badly.
/// It is what the dashboard sorts and colours by.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    Error,
    Warn,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warn => "warn",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "error" => Some(Self::Error),
            "warn" => Some(Self::Warn),
            _ => None,
        }
    }
}

/// The five things a rule can be, and the options each carries once resolved.
///
/// Defaults are filled in here rather than at the point of use, so that what
/// `GET /api/prose/rules` shows is what the analyzer ran.
#[derive(Debug, Clone, PartialEq)]
pub enum Options {
    /// A literal substring search over the extracted text, not over tokens, so
    /// it can name a single character. This repository's own em dash rule is
    /// this one.
    Forbid { literals: Vec<String> },
    /// A token **sequence** match, so `delve into` does not fire inside a word
    /// and punctuation between the tokens does not defeat it.
    Phrase { phrases: Vec<String> },
    /// The same token twice inside `within` tokens of itself.
    Echo { within: usize, ignore: Vec<String> },
    /// A run of sentences whose lengths all sit within `spread` of the run's
    /// mean, which is what drift sounds like from the outside.
    Uniformity { run: usize, spread: f64 },
    /// Two capitalised spellings close enough to be the same name written twice.
    ///
    /// The one rule nobody wrote, which is why it is the only one with an
    /// `allow` list, and why `length` exists beside it: running this over real
    /// documents produced almost nothing but initialisms and labels until both
    /// were in place.
    Consistent {
        distance: usize,
        length: usize,
        allow: Vec<String>,
    },
}

impl Options {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Forbid { .. } => "forbid",
            Self::Phrase { .. } => "phrase",
            Self::Echo { .. } => "echo",
            Self::Uniformity { .. } => "uniformity",
            Self::Consistent { .. } => "consistent",
        }
    }
}

/// One rule, with every default already resolved.
#[derive(Debug, Clone, PartialEq)]
pub struct Rule {
    pub id: String,
    pub severity: Severity,
    /// What a finding says, when the author would rather say it themselves.
    ///
    /// Absent means the rule describes its own finding, which for three of the
    /// five is the only place the arithmetic gets said in words.
    pub message: Option<String>,
    pub options: Options,
}

/// A rules file, resolved, sorted, and stamped.
#[derive(Debug, Clone, PartialEq)]
pub struct Ruleset {
    rules: Vec<Rule>,
    digest: String,
}

impl Ruleset {
    /// A wiki that has never written rules. Not an error anywhere: no rules is a
    /// state a wiki is genuinely in, and it is the answer a caller asking what
    /// the rules are should get.
    pub fn empty() -> Self {
        Self::new(Vec::new())
    }

    /// Sort by `id`, then stamp.
    ///
    /// Sorted rather than kept in file order because rule order changes nothing
    /// about the output: findings come back ordered by where they are in the
    /// text. A digest that moved when somebody reordered the file would report a
    /// change that had not happened.
    fn new(mut rules: Vec<Rule>) -> Self {
        rules.sort_by(|left, right| left.id.cmp(&right.id));
        let digest = digest_of(&normalize(&rules));

        Self { rules, digest }
    }

    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The stamp a finding quotes, `sha256:` and sixty-four hex characters.
    ///
    /// Taken over the **normalized** ruleset rather than the file's bytes, which
    /// is what makes it useful: two spellings of the same rule stamp the same,
    /// and a caller holding [`Ruleset::normalized`] can recompute it and check.
    /// A finding and a ruleset that disagree on it came from different rules,
    /// which is otherwise an invisible way for an assistant to be confidently
    /// wrong about why something fired.
    pub fn digest(&self) -> &str {
        &self.digest
    }

    /// Every rule with its options resolved, which is what goes over the wire.
    pub fn normalized(&self) -> Vec<NormalizedRule> {
        normalize(&self.rules)
    }
}

/// One rule as `GET /api/prose/rules` reports it.
///
/// Flat rather than tagged by kind, and each option present only on the rules
/// that have it. A caller reproducing a finding needs the values the analyzer
/// used, and TOML has more than one way to write most of them.
#[derive(Debug, Clone, PartialEq, Serialize, ToSchema)]
pub struct NormalizedRule {
    #[schema(example = "no-em-dash")]
    pub id: String,
    /// One of `forbid`, `phrase`, `echo`, `uniformity`, `consistent`.
    #[schema(example = "forbid")]
    pub kind: &'static str,
    /// `error` or `warn`.
    #[schema(example = "warn")]
    pub severity: &'static str,
    /// What a finding says, when the rules file gave one. Absent means the rule
    /// describes its own findings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub literals: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phrases: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 40)]
    pub within: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ignore: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 5)]
    pub run: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 3.0)]
    pub spread: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 1)]
    pub distance: Option<usize>,
    /// The shortest a spelling may be and still be taken for a name.
    #[serde(skip_serializing_if = "Option::is_none")]
    #[schema(example = 4)]
    pub length: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow: Option<Vec<String>>,
}

fn normalize(rules: &[Rule]) -> Vec<NormalizedRule> {
    rules
        .iter()
        .map(|rule| {
            let mut view = NormalizedRule {
                id: rule.id.clone(),
                kind: rule.options.kind(),
                severity: rule.severity.as_str(),
                message: rule.message.clone(),
                literals: None,
                phrases: None,
                within: None,
                ignore: None,
                run: None,
                spread: None,
                distance: None,
                length: None,
                allow: None,
            };

            match &rule.options {
                Options::Forbid { literals } => view.literals = Some(literals.clone()),
                Options::Phrase { phrases } => view.phrases = Some(phrases.clone()),
                Options::Echo { within, ignore } => {
                    view.within = Some(*within);
                    view.ignore = Some(ignore.clone());
                }
                Options::Uniformity { run, spread } => {
                    view.run = Some(*run);
                    view.spread = Some(*spread);
                }
                Options::Consistent {
                    distance,
                    length,
                    allow,
                } => {
                    view.distance = Some(*distance);
                    view.length = Some(*length);
                    view.allow = Some(allow.clone());
                }
            }

            view
        })
        .collect()
}

/// Hash the normalized ruleset exactly as it is served.
///
/// Over the JSON rather than over some private encoding, so that a caller
/// holding the response can reproduce the number rather than take it on trust.
fn digest_of(rules: &[NormalizedRule]) -> String {
    let json = serde_json::to_vec(rules).unwrap_or_default();
    format!("sha256:{:x}", Sha256::digest(&json))
}

/// One rule firing on one span.
#[derive(Debug, Clone, PartialEq)]
pub struct Finding {
    pub rule: String,
    pub severity: Severity,
    /// Byte offset into the text that was analysed.
    pub start: usize,
    pub end: usize,
    /// The text it fired on, cut from the source.
    pub quote: String,
    pub message: String,
    /// The numbers the rule compared. See the module docs: this is what makes a
    /// finding checkable rather than believable.
    pub receipt: Value,
}

/// Everything one run produced.
#[derive(Debug, Clone, PartialEq)]
pub struct Analysis {
    pub findings: Vec<Finding>,
    /// Whether [`MAX_FINDINGS`] cut the list short.
    pub truncated: bool,
}

/// Run every rule over one body.
///
/// Two rules may fire on overlapping spans and both are reported. Suppressing
/// one would mean ranking rules against each other, and the author wrote them
/// all.
///
/// Findings come back ordered by start offset, then by rule `id`, then by end,
/// so two runs over the same text produce the same list in the same order.
pub fn analyze(source: &str, ruleset: &Ruleset) -> Analysis {
    if ruleset.is_empty() {
        return Analysis {
            findings: Vec::new(),
            truncated: false,
        };
    }

    let text = text::Text::of(source);
    let mut findings: Vec<Finding> = ruleset
        .rules()
        .iter()
        .flat_map(|rule| rules::findings(rule, &text))
        .collect();

    findings.sort_by(|left, right| {
        left.start
            .cmp(&right.start)
            .then_with(|| left.rule.cmp(&right.rule))
            .then_with(|| left.end.cmp(&right.end))
            .then_with(|| left.message.cmp(&right.message))
    });

    let truncated = findings.len() > MAX_FINDINGS;
    findings.truncate(MAX_FINDINGS);

    Analysis {
        findings,
        truncated,
    }
}

#[derive(Debug, Error)]
pub enum ProseError {
    #[error("the prose rules could not be read")]
    Io {
        #[source]
        source: std::io::Error,
    },
    #[error("the prose rules are not valid UTF-8")]
    NotUtf8,
    #[error("the prose rules are not valid TOML: {message}")]
    Syntax { message: String },
    #[error("{reason}")]
    Invalid { reason: String },
}

/// Read `<wiki>/.rhizolog/prose.toml`.
///
/// A file that is not there is an **empty ruleset, not an error**. A wiki that
/// has never written rules is the ordinary case, and the two endpoints that read
/// this both have a sensible answer for it: no findings, and no rules. A file
/// that will not parse is a different thing entirely, and one somebody has just
/// made a mistake in, so it is reported as itself.
pub async fn load(wiki_root: impl AsRef<Path>) -> Result<Ruleset, ProseError> {
    let path = wiki_root.as_ref().join(INTERNAL_DIR).join(RULES_FILE);

    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let source = String::from_utf8(bytes).map_err(|_| ProseError::NotUtf8)?;
            parse(&source)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Ruleset::empty()),
        Err(source) => Err(ProseError::Io { source }),
    }
}

/// Parse a rules file, resolving every default and refusing anything ambiguous.
pub fn parse(source: &str) -> Result<Ruleset, ProseError> {
    // A byte order mark would make the first key unparseable and the error
    // unhelpful. PowerShell's `Out-File` writes one, and a BOM has already cost
    // this project one real bug by hiding a page's frontmatter.
    let source = source.strip_prefix('\u{feff}').unwrap_or(source);

    // `to_string` rather than the bare message, because TOML's error carries the
    // line and column and that is the half a caller can act on.
    let file: File = toml::from_str(source).map_err(|error| ProseError::Syntax {
        message: error.to_string(),
    })?;

    if file.rule.len() > MAX_RULES {
        return Err(ProseError::Invalid {
            reason: format!(
                "{} rules, and at most {MAX_RULES} are allowed",
                file.rule.len()
            ),
        });
    }

    let mut seen = BTreeSet::new();
    let mut rules = Vec::with_capacity(file.rule.len());

    for raw in &file.rule {
        let rule = raw.resolve()?;

        // Not a last-one-wins. A file with the same id twice is a file somebody
        // edited into a state they did not mean, and quietly keeping one of the
        // two would hide it.
        if !seen.insert(rule.id.clone()) {
            return Err(ProseError::Invalid {
                reason: format!("two rules share the id {:?}", rule.id),
            });
        }

        rules.push(rule);
    }

    Ok(Ruleset::new(rules))
}

// ------------------------------------------------------- the file, as written

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    rule: Vec<RawRule>,
}

/// Every option on every kind, all optional.
///
/// Flat rather than an enum tagged by `kind`, for one reason: this is what lets
/// an option that belongs to another kind be refused by name. `within` on a
/// `forbid` rule is a mistake somebody wants to hear about, and a tagged enum
/// would silently ignore it.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    id: String,
    kind: String,
    severity: Option<String>,
    message: Option<String>,
    literals: Option<Vec<String>>,
    phrases: Option<Vec<String>>,
    within: Option<usize>,
    ignore: Option<Vec<String>>,
    run: Option<usize>,
    spread: Option<f64>,
    distance: Option<usize>,
    length: Option<usize>,
    allow: Option<Vec<String>>,
}

/// Which options each kind owns. Anything else set on it is refused.
const OWNED: [(&str, &[&str]); 5] = [
    ("forbid", &["literals"]),
    ("phrase", &["phrases"]),
    ("echo", &["within", "ignore"]),
    ("uniformity", &["run", "spread"]),
    ("consistent", &["distance", "length", "allow"]),
];

impl RawRule {
    /// Which per-kind options this rule actually set.
    fn present(&self) -> Vec<&'static str> {
        [
            ("literals", self.literals.is_some()),
            ("phrases", self.phrases.is_some()),
            ("within", self.within.is_some()),
            ("ignore", self.ignore.is_some()),
            ("run", self.run.is_some()),
            ("spread", self.spread.is_some()),
            ("distance", self.distance.is_some()),
            ("length", self.length.is_some()),
            ("allow", self.allow.is_some()),
        ]
        .into_iter()
        .filter_map(|(name, set)| set.then_some(name))
        .collect()
    }

    fn resolve(&self) -> Result<Rule, ProseError> {
        let refuse = |reason: String| ProseError::Invalid {
            reason: format!("rule {:?}: {reason}", self.id),
        };

        if self.id.trim().is_empty() {
            return Err(ProseError::Invalid {
                reason: "a rule needs an id".to_owned(),
            });
        }

        let Some((_, owned)) = OWNED.iter().find(|(name, _)| *name == self.kind) else {
            let kinds: Vec<&str> = OWNED.iter().map(|(name, _)| *name).collect();
            return Err(refuse(format!(
                "unknown kind {:?}; one of {}",
                self.kind,
                kinds.join(", ")
            )));
        };

        for option in self.present() {
            if !owned.contains(&option) {
                return Err(refuse(format!(
                    "{option:?} is not an option of a {} rule",
                    self.kind
                )));
            }
        }

        let severity = match self.severity.as_deref() {
            None => Severity::Warn,
            Some(raw) => Severity::parse(raw)
                .ok_or_else(|| refuse(format!("severity {raw:?} is not error or warn")))?,
        };

        let options = match self.kind.as_str() {
            "forbid" => Options::Forbid {
                literals: strings(self.literals.as_deref(), "literals", &refuse)?,
            },
            "phrase" => Options::Phrase {
                phrases: strings(self.phrases.as_deref(), "phrases", &refuse)?,
            },
            "echo" => Options::Echo {
                within: bounded(
                    self.within,
                    DEFAULT_WITHIN,
                    1,
                    MAX_WITHIN,
                    "within",
                    &refuse,
                )?,
                ignore: self.ignore.clone().unwrap_or_default(),
            },
            "uniformity" => Options::Uniformity {
                run: bounded(self.run, DEFAULT_RUN, 2, MAX_RUN, "run", &refuse)?,
                spread: spread(self.spread, &refuse)?,
            },
            "consistent" => Options::Consistent {
                distance: bounded(
                    self.distance,
                    DEFAULT_DISTANCE,
                    1,
                    MAX_DISTANCE,
                    "distance",
                    &refuse,
                )?,
                length: bounded(
                    self.length,
                    DEFAULT_LENGTH,
                    1,
                    MAX_LENGTH,
                    "length",
                    &refuse,
                )?,
                allow: self.allow.clone().unwrap_or_default(),
            },
            _ => unreachable!("the kind was matched against OWNED above"),
        };

        Ok(Rule {
            id: self.id.clone(),
            severity,
            message: self.message.clone(),
            options,
        })
    }
}

/// A required, non-empty list of non-empty strings.
fn strings(
    values: Option<&[String]>,
    field: &str,
    refuse: &impl Fn(String) -> ProseError,
) -> Result<Vec<String>, ProseError> {
    let values = values.unwrap_or_default();

    if values.is_empty() {
        return Err(refuse(format!("{field} is required and cannot be empty")));
    }
    if values.iter().any(|value| value.is_empty()) {
        return Err(refuse(format!("{field} holds an empty string")));
    }

    Ok(values.to_vec())
}

fn bounded(
    value: Option<usize>,
    default: usize,
    least: usize,
    most: usize,
    field: &str,
    refuse: &impl Fn(String) -> ProseError,
) -> Result<usize, ProseError> {
    let value = value.unwrap_or(default);

    if value < least || value > most {
        return Err(refuse(format!(
            "{field} is {value}, and has to be between {least} and {most}"
        )));
    }

    Ok(value)
}

fn spread(value: Option<f64>, refuse: &impl Fn(String) -> ProseError) -> Result<f64, ProseError> {
    let value = value.unwrap_or(DEFAULT_SPREAD);

    if !value.is_finite() || !(0.0..=MAX_SPREAD).contains(&value) {
        return Err(refuse(format!(
            "spread is {value}, and has to be between 0 and {MAX_SPREAD}"
        )));
    }

    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(source: &str) -> String {
        match parse(source) {
            Ok(ruleset) => panic!("expected a refusal, got {ruleset:?}"),
            Err(error) => error.to_string(),
        }
    }

    /// What `GET /api/prose/rules` exists to answer: the values the analyzer
    /// used, not the ones the file happened to spell out.
    #[test]
    fn every_default_is_resolved_before_anything_is_served() {
        let ruleset = parse(
            "[[rule]]\nid = \"echo\"\nkind = \"echo\"\n\n\
             [[rule]]\nid = \"uniformity\"\nkind = \"uniformity\"\n\n\
             [[rule]]\nid = \"names\"\nkind = \"consistent\"\n",
        )
        .expect("the rules parse");

        let normalized = ruleset.normalized();
        // Sorted by id, so the order does not depend on the file.
        assert_eq!(
            normalized
                .iter()
                .map(|rule| rule.id.as_str())
                .collect::<Vec<_>>(),
            ["echo", "names", "uniformity"]
        );

        assert_eq!(normalized[0].within, Some(DEFAULT_WITHIN));
        assert_eq!(normalized[0].ignore, Some(Vec::new()));
        assert_eq!(normalized[0].severity, "warn");
        assert_eq!(normalized[1].distance, Some(DEFAULT_DISTANCE));
        assert_eq!(normalized[1].length, Some(DEFAULT_LENGTH));
        assert_eq!(normalized[2].run, Some(DEFAULT_RUN));
        assert_eq!(normalized[2].spread, Some(DEFAULT_SPREAD));

        // Only its own options, so nothing suggests a rule reads a value it
        // never looks at.
        assert!(normalized[0].literals.is_none());
        assert!(normalized[0].run.is_none());
    }

    /// The digest is over the resolved rules, which is what makes it worth
    /// quoting: a caller reproducing a finding needs the values the analyzer
    /// used, and TOML has more than one way to write most of them.
    #[test]
    fn two_spellings_of_the_same_rules_stamp_the_same() {
        let terse = parse("[[rule]]\nid = \"echo\"\nkind = \"echo\"\n").expect("parses");
        let spelled_out = parse(
            "[[rule]]\nid = \"echo\"\nkind = \"echo\"\nseverity = \"warn\"\n\
             within = 40\nignore = []\n",
        )
        .expect("parses");

        assert_eq!(terse.digest(), spelled_out.digest());

        // And so does the same file with its rules the other way round, because
        // rule order changes nothing about what comes back.
        let one_way = parse(
            "[[rule]]\nid = \"a\"\nkind = \"forbid\"\nliterals = [\"x\"]\n\n\
             [[rule]]\nid = \"b\"\nkind = \"forbid\"\nliterals = [\"y\"]\n",
        )
        .expect("parses");
        let other_way = parse(
            "[[rule]]\nid = \"b\"\nkind = \"forbid\"\nliterals = [\"y\"]\n\n\
             [[rule]]\nid = \"a\"\nkind = \"forbid\"\nliterals = [\"x\"]\n",
        )
        .expect("parses");
        assert_eq!(one_way.digest(), other_way.digest());
    }

    #[test]
    fn changing_a_value_changes_the_stamp() {
        let before = parse("[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 40\n").unwrap();
        let after = parse("[[rule]]\nid = \"echo\"\nkind = \"echo\"\nwithin = 41\n").unwrap();

        assert_ne!(before.digest(), after.digest());
        assert!(before.digest().starts_with("sha256:"));
        assert_eq!(before.digest().len(), "sha256:".len() + 64);
    }

    /// A caller holding the response can recompute the number rather than take
    /// it on trust, which is the only thing that makes it useful.
    #[test]
    fn the_stamp_is_over_exactly_what_is_served() {
        let ruleset = parse("[[rule]]\nid = \"echo\"\nkind = \"echo\"\n").unwrap();
        let json = serde_json::to_vec(&ruleset.normalized()).unwrap();

        assert_eq!(
            ruleset.digest(),
            format!("sha256:{:x}", Sha256::digest(&json))
        );
    }

    /// Not a last-one-wins: a file with the same id twice is one somebody edited
    /// into a state they did not mean.
    #[test]
    fn two_rules_may_not_share_an_id() {
        let reason = refused(
            "[[rule]]\nid = \"echo\"\nkind = \"echo\"\n\n\
             [[rule]]\nid = \"echo\"\nkind = \"forbid\"\nliterals = [\"x\"]\n",
        );
        assert!(reason.contains("share the id"), "got {reason}");
    }

    #[test]
    fn an_unknown_kind_names_the_ones_that_exist() {
        let reason = refused("[[rule]]\nid = \"a\"\nkind = \"speling\"\n");

        assert!(reason.contains("unknown kind"), "got {reason}");
        for kind in ["forbid", "phrase", "echo", "uniformity", "consistent"] {
            assert!(reason.contains(kind), "{kind} is missing from {reason}");
        }
    }

    /// The reason the file is read into one flat struct rather than an enum
    /// tagged by kind: a tagged enum would ignore this quietly.
    #[test]
    fn an_option_belonging_to_another_kind_is_a_mistake_worth_hearing_about() {
        let reason =
            refused("[[rule]]\nid = \"a\"\nkind = \"forbid\"\nliterals = [\"x\"]\nwithin = 4\n");
        assert!(reason.contains("\"within\""), "got {reason}");
        assert!(reason.contains("forbid"), "got {reason}");
    }

    #[test]
    fn a_misspelled_field_is_refused_rather_than_ignored() {
        assert!(
            refused("[[rule]]\nid = \"a\"\nkind = \"echo\"\nwitin = 4\n").contains("witin"),
            "an unknown field has to name itself"
        );
    }

    #[test]
    fn a_rule_that_names_nothing_to_look_for_is_refused() {
        assert!(refused("[[rule]]\nid = \"a\"\nkind = \"forbid\"\n").contains("required"));
        assert!(
            refused("[[rule]]\nid = \"a\"\nkind = \"forbid\"\nliterals = []\n")
                .contains("required")
        );
        assert!(
            refused("[[rule]]\nid = \"a\"\nkind = \"phrase\"\nphrases = [\"\"]\n")
                .contains("empty string")
        );
    }

    /// Limits are numbers. `echo` costs a scan per token times `within`, and
    /// `consistent` at distance ten would call every capitalised word a
    /// misspelling of every other.
    #[test]
    fn an_option_outside_its_range_is_refused() {
        for (source, field) in [
            (
                "[[rule]]\nid = \"a\"\nkind = \"echo\"\nwithin = 0\n",
                "within",
            ),
            (
                "[[rule]]\nid = \"a\"\nkind = \"echo\"\nwithin = 100000\n",
                "within",
            ),
            (
                "[[rule]]\nid = \"a\"\nkind = \"uniformity\"\nrun = 1\n",
                "run",
            ),
            (
                "[[rule]]\nid = \"a\"\nkind = \"uniformity\"\nspread = -1.0\n",
                "spread",
            ),
            (
                "[[rule]]\nid = \"a\"\nkind = \"consistent\"\ndistance = 9\n",
                "distance",
            ),
            (
                "[[rule]]\nid = \"a\"\nkind = \"consistent\"\nlength = 0\n",
                "length",
            ),
        ] {
            assert!(
                refused(source).contains(field),
                "{field} was allowed through"
            );
        }
    }

    #[test]
    fn a_file_that_will_not_parse_is_reported_as_itself() {
        assert!(matches!(
            parse("[[rule]\nid = \"a\"\n"),
            Err(ProseError::Syntax { .. })
        ));
    }

    /// A BOM has already cost this project one real bug, by hiding a page's
    /// frontmatter. PowerShell's `Out-File` writes one.
    #[test]
    fn a_byte_order_mark_does_not_hide_the_first_rule() {
        let ruleset = parse("\u{feff}[[rule]]\nid = \"echo\"\nkind = \"echo\"\n")
            .expect("a BOM is not a syntax error");
        assert_eq!(ruleset.rules().len(), 1);
    }

    #[tokio::test]
    async fn a_wiki_with_no_rules_file_has_an_empty_ruleset_rather_than_an_error() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let ruleset = load(directory.path())
            .await
            .expect("no file is not an error");

        assert!(ruleset.is_empty());
        assert!(ruleset.normalized().is_empty());
        // And it still stamps, so a caller comparing digests has something to
        // compare rather than a special case.
        assert!(ruleset.digest().starts_with("sha256:"));
    }

    #[tokio::test]
    async fn a_rules_file_is_read_from_the_wiki_it_belongs_to() {
        let directory = tempfile::tempdir().expect("a temporary directory");
        let internal = directory.path().join(INTERNAL_DIR);
        std::fs::create_dir_all(&internal).expect("the internal directory");
        std::fs::write(
            internal.join(RULES_FILE),
            "[[rule]]\nid = \"no-em-dash\"\nkind = \"forbid\"\nliterals = [\"\\u2014\"]\n",
        )
        .expect("the rules file");

        let ruleset = load(directory.path()).await.expect("the rules parse");
        assert_eq!(ruleset.rules().len(), 1);
        assert_eq!(ruleset.rules()[0].id, "no-em-dash");
    }
}
