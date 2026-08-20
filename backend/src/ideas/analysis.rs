//! Lexical analysis, version 1: `tfidf/v1`.
//!
//! This is the whole of what Rhizolog means by "notices which ideas keep coming
//! back". It is TF-IDF over one owner's captures and a cosine similarity, with
//! no model, no service, no network request and nothing learned. Everything it
//! reports is a word the user typed.
//!
//! ## Why there is no stemming and no stop-word list
//!
//! Both would improve recall and both would break the promise. A signal shown to
//! the user has to be a term that appears literally in the text on both sides,
//! because the receipt says "these words are why" and a stem is not a word
//! anybody wrote. TF-IDF already downweights whatever is common across the
//! corpus, which is the job a stop-word list would be doing, and it does it from
//! the user's own writing rather than from a list somebody else made.
//!
//! Bigrams are here for the same reason: `dungeon seeds` matching `dungeon
//! seeds` is worth more than two separate word matches, and it is still a phrase
//! the user can see in their own capture.
//!
//! ## The contributions add up to the score
//!
//! Both vectors are unit length, so the cosine is their dot product, and a dot
//! product is a sum of per-term contributions. That is not a presentational
//! convenience, it is the property that makes a candidate explainable: the
//! shared signals a response lists are the actual terms of the actual sum, and
//! [`Candidate::explained`] says how much of the score the listed ones account
//! for.
//!
//! ## Changing anything here is a version change
//!
//! Tokenization, weighting, the threshold and how a centroid is built are the
//! definition of [`ANALYZER`]. Changing one of them changes every stored fixture
//! and every number a user has already seen, so it comes with a new version
//! string and a note on `knowledge-base/idea-inbox.md`.

use std::collections::{BTreeMap, BTreeSet};

use crate::ideas::{CaptureId, IdeaId};

/// The name this analyzer answers to, returned with every candidate response.
pub const ANALYZER: &str = "tfidf/v1";

/// How similar a capture and a target have to be before it is worth suggesting.
///
/// Below this, the shared terms are usually one common word and the suggestion
/// is noise. It is a lexical-similarity cutoff and not a probability that the
/// two thoughts are related, which is also how the dashboard has to describe it.
pub const THRESHOLD: f64 = 0.35;

/// How many candidates one request may return.
///
/// Three, because the user is being asked a question about each one and a list
/// long enough to skim is a list nobody reads.
pub const MAX_CANDIDATES: usize = 3;

/// How many shared signals accompany one candidate.
pub const MAX_SIGNALS: usize = 5;

/// What joins the two halves of a bigram.
///
/// A space, which is a separator a unigram cannot contain: tokens are runs of
/// alphanumeric characters, so nothing else can produce `dungeon seeds`. It is
/// also readable, which matters because these strings are shown to the user.
pub const BIGRAM: char = ' ';

/// Split text into the terms this analyzer counts: unigrams, then the bigrams
/// of adjacent words.
///
/// Lowercasing happens to the whole string before splitting rather than to each
/// word afterwards, and the order is not arbitrary. Rust's Unicode lowercase
/// conversion can turn one character into several, some of which are combining
/// marks and therefore not alphanumeric, so lowercasing first is what lets the
/// split see them.
pub fn terms(text: &str) -> Vec<String> {
    let lowered = text.to_lowercase();
    let words: Vec<&str> = lowered
        .split(|character: char| !character.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .collect();

    let mut terms: Vec<String> = words.iter().map(|word| (*word).to_owned()).collect();
    terms.extend(
        words
            .windows(2)
            .map(|pair| format!("{}{BIGRAM}{}", pair[0], pair[1])),
    );
    terms
}

/// The same terms, counted.
pub fn counts(text: &str) -> BTreeMap<String, u32> {
    let mut counts = BTreeMap::new();
    for term in terms(text) {
        *counts.entry(term).or_insert(0) += 1;
    }
    counts
}

/// One capture's terms, as the index holds them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Document {
    pub capture: CaptureId,
    pub terms: BTreeMap<String, u32>,
}

/// One owner's captures, and how often each term appears across them.
///
/// The corpus is *the owner's*, never the wiki's. A weight computed over
/// somebody else's writing would make the score a channel out of their inbox:
/// how rare a word is across a corpus says something about the corpus.
#[derive(Debug, Clone, Default)]
pub struct Corpus {
    documents: BTreeMap<CaptureId, BTreeMap<String, u32>>,
    frequencies: BTreeMap<String, usize>,
}

impl Corpus {
    /// Build a corpus from every one of an owner's captures.
    ///
    /// A capture with no terms at all still belongs here: it is one of the `N`
    /// the idf is computed over, and leaving it out would make the weights
    /// depend on whether somebody had once saved a line of punctuation.
    pub fn new(documents: impl IntoIterator<Item = Document>) -> Self {
        let documents: BTreeMap<CaptureId, BTreeMap<String, u32>> = documents
            .into_iter()
            .map(|document| (document.capture, document.terms))
            .collect();

        let mut frequencies: BTreeMap<String, usize> = BTreeMap::new();
        for terms in documents.values() {
            for term in terms.keys() {
                *frequencies.entry(term.clone()).or_insert(0) += 1;
            }
        }

        Self {
            documents,
            frequencies,
        }
    }

    /// `N`: how many captures the weights are computed over.
    pub fn len(&self) -> usize {
        self.documents.len()
    }

    pub fn is_empty(&self) -> bool {
        self.documents.is_empty()
    }

    pub fn contains(&self, capture: &CaptureId) -> bool {
        self.documents.contains_key(capture)
    }

    /// Every capture in the corpus, in id order and therefore in the order they
    /// were written.
    pub fn captures(&self) -> impl Iterator<Item = &CaptureId> {
        self.documents.keys()
    }

    /// How many distinct terms one capture has, or zero for one that is not
    /// here.
    pub fn distinct_terms(&self, capture: &CaptureId) -> usize {
        self.documents.get(capture).map_or(0, BTreeMap::len)
    }

    /// How many of the owner's captures contain this term.
    pub fn frequency(&self, term: &str) -> usize {
        self.frequencies.get(term).copied().unwrap_or(0)
    }

    /// `ln((1 + N) / (1 + documents containing the term)) + 1`.
    ///
    /// Both halves are smoothed, and the `+ 1` is a floor rather than a
    /// rounding: a term present in every capture gets an idf of exactly 1 rather
    /// than 0, so it is downweighted relative to a rare word without being
    /// erased. Two captures made entirely of the owner's most ordinary
    /// vocabulary can therefore still score highly against each other, which is
    /// the right answer when they really do say the same ordinary thing, and is
    /// why the threshold and the three-candidate cap exist.
    pub fn idf(&self, term: &str) -> f64 {
        let documents = self.len() as f64;
        let containing = self.frequency(term) as f64;
        ((1.0 + documents) / (1.0 + containing)).ln() + 1.0
    }

    /// One capture's unit-length TF-IDF vector.
    ///
    /// `None` for a capture that is not in the corpus, and for one whose text
    /// holds no terms at all: a line of punctuation has no direction to compare
    /// anything against, and dividing by its length is the arithmetic saying so.
    pub fn vector(&self, capture: &CaptureId) -> Option<Vector> {
        let terms = self.documents.get(capture)?;

        let total: u32 = terms.values().copied().sum();
        if total == 0 {
            return None;
        }
        let total = f64::from(total);

        let weights = terms
            .iter()
            .map(|(term, occurrences)| {
                let frequency = f64::from(*occurrences) / total;
                (term.clone(), frequency * self.idf(term))
            })
            .collect();

        Vector::unit(weights)
    }
}

/// A capture's weighted terms, always unit length.
///
/// Unit length by construction rather than by convention, because that is what
/// makes [`Vector::similarity`] a plain dot product and therefore makes
/// [`Vector::shared`] an exact decomposition of it.
#[derive(Debug, Clone, PartialEq)]
pub struct Vector {
    weights: BTreeMap<String, f64>,
}

impl Vector {
    fn unit(weights: BTreeMap<String, f64>) -> Option<Self> {
        let norm = weights.values().map(|weight| weight * weight).sum::<f64>();
        let norm = norm.sqrt();
        if norm <= 0.0 || !norm.is_finite() {
            return None;
        }

        Some(Self {
            weights: weights
                .into_iter()
                .map(|(term, weight)| (term, weight / norm))
                .collect(),
        })
    }

    /// An idea's centroid: the mean of its captures' unit vectors, made unit
    /// length itself.
    ///
    /// The mean rather than the sum, so that a thread of ten captures and a
    /// thread of two are the same kind of thing. Normalising afterwards makes no
    /// difference to the cosine, and it does make one to [`Vector::shared`]:
    /// contributions only sum to the similarity if both sides are unit length.
    pub fn centroid<'a>(vectors: impl IntoIterator<Item = &'a Self>) -> Option<Self> {
        let mut sum: BTreeMap<String, f64> = BTreeMap::new();
        let mut count = 0usize;

        for vector in vectors {
            count += 1;
            for (term, weight) in &vector.weights {
                *sum.entry(term.clone()).or_insert(0.0) += weight;
            }
        }

        if count == 0 {
            return None;
        }

        let count = count as f64;
        let mean = sum
            .into_iter()
            .map(|(term, weight)| (term, weight / count))
            .collect();
        Self::unit(mean)
    }

    pub fn weight(&self, term: &str) -> f64 {
        self.weights.get(term).copied().unwrap_or(0.0)
    }

    pub fn is_empty(&self) -> bool {
        self.weights.is_empty()
    }

    /// Cosine similarity, which for two unit vectors is their dot product.
    ///
    /// Summed in term order, which is the order [`Vector::shared`] finds the
    /// same products in, so the two agree to the bit.
    pub fn similarity(&self, other: &Self) -> f64 {
        self.weights
            .iter()
            .map(|(term, weight)| weight * other.weight(term))
            .sum()
    }

    /// The terms both vectors carry, biggest contribution to the similarity
    /// first.
    ///
    /// Every contribution is `this weight * that weight`, and their sum is
    /// exactly [`Vector::similarity`]. Ties break on the term so that the same
    /// corpus always answers the same way.
    pub fn shared(&self, other: &Self) -> Vec<Signal> {
        let mut signals: Vec<Signal> = self
            .weights
            .iter()
            .filter_map(|(term, weight)| {
                let target = other.weight(term);
                (target > 0.0).then(|| Signal {
                    term: term.clone(),
                    capture_weight: *weight,
                    target_weight: target,
                    contribution: weight * target,
                })
            })
            .collect();

        signals.sort_by(|first, second| {
            second
                .contribution
                .total_cmp(&first.contribution)
                .then_with(|| first.term.cmp(&second.term))
        });
        signals
    }
}

/// One term two records have in common, and what it was worth.
#[derive(Debug, Clone, PartialEq)]
pub struct Signal {
    /// A word, or two adjacent words, appearing literally in both.
    pub term: String,
    /// Its weight in the capture being analysed.
    pub capture_weight: f64,
    /// Its weight in the target.
    pub target_weight: f64,
    /// The product of the two. These sum to the similarity.
    pub contribution: f64,
}

/// One idea thread, as candidate selection sees it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    pub id: IdeaId,
    pub name: String,
    pub retired: bool,
    /// The captures it currently holds and can still read.
    pub members: Vec<CaptureId>,
}

/// Everything candidate selection reads, gathered before any of it is scored.
///
/// A plain value rather than a handle to the index, so the rules below are a
/// pure function of stated inputs and a test can write down a corpus of four
/// captures and the exact numbers it expects back.
#[derive(Debug, Clone, Default)]
pub struct Field {
    pub corpus: Corpus,
    pub threads: Vec<Thread>,
    /// Idea candidates this owner has turned down, until they reconsider.
    pub rejected_candidates: BTreeSet<(IdeaId, CaptureId)>,
    /// Capture pairs this owner has turned down, in canonical order.
    pub rejected_pairs: BTreeSet<(CaptureId, CaptureId)>,
}

/// What a candidate suggests connecting to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// An existing thread. Accepting appends one membership event.
    Idea {
        id: IdeaId,
        name: String,
        members: usize,
    },
    /// Another loose capture. Accepting creates a user-named idea holding both,
    /// which is how a thread comes to exist before there is a thread.
    Capture { id: CaptureId },
}

impl Target {
    pub fn id(&self) -> &str {
        match self {
            Self::Idea { id, .. } => id.as_str(),
            Self::Capture { id } => id.as_str(),
        }
    }

    /// The tie-break between two candidates that scored the same: a thread that
    /// already exists before a loose capture, then by id. Connecting to a thread
    /// is the smaller action, because it does not ask the user to name anything.
    fn order(&self) -> (u8, &str) {
        match self {
            Self::Idea { id, .. } => (0, id.as_str()),
            Self::Capture { id } => (1, id.as_str()),
        }
    }
}

/// One suggestion, and the evidence for it.
#[derive(Debug, Clone, PartialEq)]
pub struct Candidate {
    pub target: Target,
    pub similarity: f64,
    /// The shared terms, biggest first, at most [`MAX_SIGNALS`] of them.
    pub signals: Vec<Signal>,
}

impl Candidate {
    /// How much of the similarity the listed signals account for.
    ///
    /// Less than [`Candidate::similarity`] whenever more than [`MAX_SIGNALS`]
    /// terms were shared. Saying so is the difference between showing the
    /// evidence and implying it is all of it.
    pub fn explained(&self) -> f64 {
        self.signals.iter().map(|signal| signal.contribution).sum()
    }
}

/// Suggest up to [`MAX_CANDIDATES`] connections for one capture.
///
/// Scored against every non-retired thread and every *unthreaded* capture the
/// owner has. Unthreaded means belonging to no live thread: suggesting a capture
/// that is already in one would be suggesting a grouping that exists, and the
/// thread itself is the better target for that. It also happens to be why the
/// plan's "exclude captures that already share an idea with it" needs no
/// separate check, since a capture in no thread shares none.
///
/// Archived captures stay eligible on purpose. Rediscovering something put aside
/// months ago is the feature, not an edge case.
///
/// This is advisory and it stays advisory: nothing here writes an event, and
/// every candidate is a question for the user to answer.
pub fn candidates(field: &Field, capture: &CaptureId) -> Vec<Candidate> {
    let Some(source) = field.corpus.vector(capture) else {
        return Vec::new();
    };

    let threaded: BTreeSet<CaptureId> = field
        .threads
        .iter()
        .filter(|thread| !thread.retired)
        .flat_map(|thread| thread.members.iter().cloned())
        .collect();

    let mut found = Vec::new();

    for thread in &field.threads {
        if thread.retired || thread.members.contains(capture) {
            continue;
        }
        if field
            .rejected_candidates
            .contains(&(thread.id.clone(), capture.clone()))
        {
            continue;
        }

        // A member whose file is gone contributes nothing rather than a zero
        // vector, and a thread with no readable member at all is not scored: a
        // centroid of nothing is not a direction.
        let vectors: Vec<Vector> = thread
            .members
            .iter()
            .filter_map(|member| field.corpus.vector(member))
            .collect();
        let Some(centroid) = Vector::centroid(vectors.iter()) else {
            continue;
        };

        let similarity = source.similarity(&centroid);
        if similarity < THRESHOLD {
            continue;
        }

        found.push(Candidate {
            target: Target::Idea {
                id: thread.id.clone(),
                name: thread.name.clone(),
                members: thread.members.len(),
            },
            similarity,
            signals: source.shared(&centroid),
        });
    }

    for other in field.corpus.captures() {
        if other == capture || threaded.contains(other) {
            continue;
        }
        if field.rejected_pairs.contains(&canonical(capture, other)) {
            continue;
        }
        let Some(vector) = field.corpus.vector(other) else {
            continue;
        };

        let similarity = source.similarity(&vector);
        if similarity < THRESHOLD {
            continue;
        }

        found.push(Candidate {
            target: Target::Capture { id: other.clone() },
            similarity,
            signals: source.shared(&vector),
        });
    }

    found.sort_by(|first, second| {
        second
            .similarity
            .total_cmp(&first.similarity)
            .then_with(|| first.target.order().cmp(&second.target.order()))
    });
    found.truncate(MAX_CANDIDATES);
    for candidate in &mut found {
        candidate.signals.truncate(MAX_SIGNALS);
    }
    found
}

/// Two captures in the order that makes their pair one thing.
///
/// The same order [`crate::ideas::Subject::pair`] writes into an event file,
/// which is what lets a rejection recorded from either side suppress the
/// suggestion from both.
fn canonical(first: &CaptureId, second: &CaptureId) -> (CaptureId, CaptureId) {
    if second < first {
        (second.clone(), first.clone())
    } else {
        (first.clone(), second.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ideas::Subject;

    fn capture_id(raw: &str) -> CaptureId {
        CaptureId::parse(raw).expect("valid capture id")
    }

    fn idea_id(raw: &str) -> IdeaId {
        IdeaId::parse(raw).expect("valid idea id")
    }

    /// Captures numbered from one, so a fixture reads as `capture(1)`.
    fn capture(n: u32) -> CaptureId {
        capture_id(&format!("20260820T1415{n:02}-000000000"))
    }

    fn document(capture: CaptureId, text: &str) -> Document {
        Document {
            capture,
            terms: counts(text),
        }
    }

    fn near(found: f64, expected: f64) {
        assert!(
            (found - expected).abs() < 1e-9,
            "expected {expected}, found {found}"
        );
    }

    #[test]
    fn tokenizes_into_unigrams_then_adjacent_bigrams() {
        assert_eq!(
            terms("Dungeon seeds again"),
            ["dungeon", "seeds", "again", "dungeon seeds", "seeds again"]
        );
    }

    /// Nothing that is not a letter or a digit survives, and nothing is stemmed
    /// or expanded on the way through.
    #[test]
    fn splits_on_everything_that_is_not_alphanumeric() {
        assert_eq!(
            terms("Rust-lang, don't!"),
            ["rust", "lang", "don", "t", "rust lang", "lang don", "don t"]
        );
        assert_eq!(
            terms("v2 costs 3"),
            ["v2", "costs", "3", "v2 costs", "costs 3"]
        );
    }

    #[test]
    fn a_single_word_makes_no_bigram() {
        assert_eq!(terms("Seeds"), ["seeds"]);
    }

    #[test]
    fn text_with_no_letters_or_digits_has_no_terms() {
        assert!(terms("...   ---   ").is_empty());
        assert!(terms("").is_empty());
    }

    #[test]
    fn counts_repeats() {
        let counted = counts("seeds seeds seeds");
        assert_eq!(counted["seeds"], 3);
        assert_eq!(counted["seeds seeds"], 2);
    }

    /// Unicode lowercasing, and the reason it happens before the split.
    #[test]
    fn lowercases_before_splitting() {
        assert_eq!(terms("STRASSE"), ["strasse"]);
        assert_eq!(terms("Ünlü"), ["ünlü"]);
    }

    /// The arithmetic, written out by hand. Two captures, one term shared.
    #[test]
    fn idf_is_smoothed_and_never_reaches_zero() {
        let corpus = Corpus::new([
            document(capture(1), "seeds"),
            document(capture(2), "seeds dungeon"),
        ]);

        assert_eq!(corpus.len(), 2);
        // In both: ln(3/3) + 1.
        assert_eq!(corpus.frequency("seeds"), 2);
        near(corpus.idf("seeds"), 1.0);
        // In one: ln(3/2) + 1.
        near(corpus.idf("dungeon"), (3.0f64 / 2.0).ln() + 1.0);
        // In none, which is what an unseen term is worth: ln(3/1) + 1.
        near(corpus.idf("unheard"), 3.0f64.ln() + 1.0);
    }

    /// One capture, one term, checked all the way through: tf, idf, weight and
    /// the unit vector that comes out.
    #[test]
    fn a_vector_is_tf_times_idf_made_unit_length() {
        let corpus = Corpus::new([
            document(capture(1), "seeds dungeon"),
            document(capture(2), "seeds loot"),
        ]);
        let vector = corpus.vector(&capture(1)).expect("a vector");

        // Terms are `seeds`, `dungeon` and `seeds dungeon`, so tf is 1/3 each.
        let seeds = (1.0 / 3.0) * 1.0;
        let dungeon = (1.0 / 3.0) * ((3.0f64 / 2.0).ln() + 1.0);
        let bigram = dungeon;
        let norm = (seeds * seeds + dungeon * dungeon + bigram * bigram).sqrt();

        near(vector.weight("seeds"), seeds / norm);
        near(vector.weight("dungeon"), dungeon / norm);
        near(vector.weight("seeds dungeon"), bigram / norm);
        near(vector.weight("loot"), 0.0);
        near(vector.similarity(&vector), 1.0);
    }

    #[test]
    fn a_capture_with_no_terms_has_no_vector() {
        let corpus = Corpus::new([document(capture(1), "..."), document(capture(2), "seeds")]);

        assert!(corpus.contains(&capture(1)));
        assert_eq!(corpus.distinct_terms(&capture(1)), 0);
        assert!(corpus.vector(&capture(1)).is_none());
        // And it still counts toward `N`, because it is still a capture.
        assert_eq!(corpus.len(), 2);
    }

    #[test]
    fn a_capture_outside_the_corpus_has_no_vector() {
        let corpus = Corpus::new([document(capture(1), "seeds")]);
        assert!(corpus.vector(&capture(9)).is_none());
    }

    #[test]
    fn identical_captures_score_one() {
        let corpus = Corpus::new([
            document(capture(1), "Dungeon seeds again."),
            document(capture(2), "Dungeon seeds again."),
        ]);

        let first = corpus.vector(&capture(1)).expect("a vector");
        let second = corpus.vector(&capture(2)).expect("a vector");
        near(first.similarity(&second), 1.0);
    }

    #[test]
    fn captures_with_nothing_in_common_score_zero() {
        let corpus = Corpus::new([
            document(capture(1), "Dungeon seeds"),
            document(capture(2), "Compiler passes"),
        ]);

        let first = corpus.vector(&capture(1)).expect("a vector");
        let second = corpus.vector(&capture(2)).expect("a vector");
        near(first.similarity(&second), 0.0);
        assert!(first.shared(&second).is_empty());
    }

    /// The property the whole receipt rests on: the listed contributions are the
    /// terms of the sum, not a summary of it.
    #[test]
    fn shared_contributions_sum_to_the_similarity() {
        let corpus = Corpus::new([
            document(capture(1), "Dungeon seeds should decide loot"),
            document(capture(2), "Seeds decide dungeon layout"),
            document(capture(3), "Compiler passes"),
        ]);

        let first = corpus.vector(&capture(1)).expect("a vector");
        let second = corpus.vector(&capture(2)).expect("a vector");
        let signals = first.shared(&second);

        assert!(!signals.is_empty());
        near(
            signals
                .iter()
                .map(|signal| signal.contribution)
                .sum::<f64>(),
            first.similarity(&second),
        );
        for signal in &signals {
            near(
                signal.contribution,
                signal.capture_weight * signal.target_weight,
            );
        }
    }

    #[test]
    fn shared_signals_come_back_biggest_first() {
        let corpus = Corpus::new([
            document(capture(1), "seeds seeds seeds dungeon"),
            document(capture(2), "seeds dungeon"),
            document(capture(3), "dungeon dungeon dungeon"),
        ]);

        let first = corpus.vector(&capture(1)).expect("a vector");
        let second = corpus.vector(&capture(2)).expect("a vector");
        let signals = first.shared(&second);

        for pair in signals.windows(2) {
            assert!(
                pair[0].contribution >= pair[1].contribution,
                "{:?} came before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    /// A centroid of one is that one, which is what makes a thread of a single
    /// capture behave exactly like the capture.
    #[test]
    fn a_centroid_of_one_vector_is_that_vector() {
        let corpus = Corpus::new([
            document(capture(1), "Dungeon seeds"),
            document(capture(2), "Compiler passes"),
        ]);
        let only = corpus.vector(&capture(1)).expect("a vector");
        let centroid = Vector::centroid(std::slice::from_ref(&only)).expect("a centroid");

        assert_eq!(centroid, only);
    }

    #[test]
    fn a_centroid_of_nothing_is_nothing() {
        assert!(Vector::centroid(std::iter::empty()).is_none());
    }

    /// A thread holding two unrelated captures points between them, so a capture
    /// matching one of them matches the thread less well than it matches that
    /// capture alone. That is the intended behaviour and it is worth pinning.
    #[test]
    fn a_centroid_sits_between_its_captures() {
        let corpus = Corpus::new([
            document(capture(1), "Dungeon seeds"),
            document(capture(2), "Compiler passes"),
            document(capture(3), "Dungeon seeds again"),
        ]);

        let members = [
            corpus.vector(&capture(1)).expect("a vector"),
            corpus.vector(&capture(2)).expect("a vector"),
        ];
        let centroid = Vector::centroid(members.iter()).expect("a centroid");
        let asking = corpus.vector(&capture(3)).expect("a vector");

        let against_thread = asking.similarity(&centroid);
        let against_capture = asking.similarity(&members[0]);
        assert!(
            against_thread < against_capture,
            "{against_thread} should be below {against_capture}"
        );
        assert!(against_thread > 0.0);
    }

    /// One pair, one identity, and it has to be the identity an event file
    /// records or a rejection made from one side would not suppress the other.
    #[test]
    fn a_pair_is_ordered_the_way_an_event_records_it() {
        let (first, second) = canonical(&capture(2), &capture(1));

        assert_eq!(first, capture(1));
        assert_eq!(second, capture(2));
        assert_eq!(
            Subject::pair(capture(2), capture(1)),
            Subject::CapturePair {
                capture: first,
                other: second,
            }
        );
    }

    /// Bigrams roughly halve what two captures can score when they share words
    /// but not phrases, which is most of what [`THRESHOLD`] is calibrated
    /// against. Worth knowing before reading 0.35 as "a third alike".
    #[test]
    fn shuffling_the_word_order_costs_about_half_the_score() {
        let corpus = Corpus::new([
            document(capture(1), "dungeon seeds decide"),
            document(capture(2), "decide seeds dungeon"),
        ]);

        let first = corpus.vector(&capture(1)).expect("a vector");
        let second = corpus.vector(&capture(2)).expect("a vector");
        let similarity = first.similarity(&second);

        // Every unigram matches and neither bigram does.
        assert_eq!(first.shared(&second).len(), 3);
        assert!(
            (0.40..0.45).contains(&similarity),
            "the same three words in a different order scored {similarity}"
        );
        near(first.similarity(&first), 1.0);
    }

    /// A corpus of four captures and one thread, used by the candidate tests
    /// below. Captures 1, 3 and 4 are one thought written three times, which is
    /// what the feature exists to notice; capture 2 is about compilers and is
    /// nowhere near any of them.
    fn field() -> Field {
        Field {
            corpus: Corpus::new([
                document(capture(1), "Dungeon seeds should decide the loot."),
                document(capture(2), "The compiler passes run in order."),
                document(capture(3), "Dungeon seeds should decide the layout."),
                document(capture(4), "Dungeon seeds should decide the rooms."),
            ]),
            ..Field::default()
        }
    }

    fn threaded_field() -> Field {
        Field {
            threads: vec![Thread {
                id: idea_id("20260820T142000-000000000"),
                name: "Dungeon seeds".to_owned(),
                retired: false,
                members: vec![capture(1), capture(3)],
            }],
            ..field()
        }
    }

    #[test]
    fn suggests_the_thread_and_leaves_its_members_out_of_the_loose_list() {
        let found = candidates(&threaded_field(), &capture(4));

        assert_eq!(found.len(), 1, "{found:#?}");
        assert_eq!(
            found[0].target,
            Target::Idea {
                id: idea_id("20260820T142000-000000000"),
                name: "Dungeon seeds".to_owned(),
                members: 2,
            }
        );
        assert!(found[0].similarity >= THRESHOLD);
        near(
            found[0].signals.iter().map(|s| s.contribution).sum::<f64>(),
            found[0].explained(),
        );
    }

    /// With no thread yet, two loose captures suggest each other, which is how a
    /// thread comes to exist at all.
    #[test]
    fn suggests_loose_captures_when_there_is_no_thread() {
        let found = candidates(&field(), &capture(4));

        assert!(!found.is_empty(), "{found:#?}");
        for candidate in &found {
            assert!(matches!(candidate.target, Target::Capture { .. }));
            assert!(candidate.similarity >= THRESHOLD);
        }
        // The compiler capture shares nothing but `the`, so it is nowhere near.
        assert!(
            !found
                .iter()
                .any(|candidate| candidate.target.id() == capture(2).as_str()),
            "{found:#?}"
        );
    }

    #[test]
    fn never_suggests_the_capture_itself() {
        for candidate in candidates(&field(), &capture(4)) {
            assert_ne!(candidate.target.id(), capture(4).as_str());
        }
    }

    #[test]
    fn a_thread_that_already_holds_the_capture_is_not_suggested() {
        let mut field = threaded_field();
        field.threads[0].members.push(capture(4));

        assert!(candidates(&field, &capture(4)).is_empty());
    }

    #[test]
    fn a_retired_thread_is_not_suggested_and_frees_its_captures() {
        let mut field = threaded_field();
        field.threads[0].retired = true;

        let found = candidates(&field, &capture(4));

        assert!(!found.is_empty());
        for candidate in &found {
            assert!(
                matches!(candidate.target, Target::Capture { .. }),
                "a retired thread was suggested: {candidate:#?}"
            );
        }
    }

    #[test]
    fn a_rejected_thread_stays_rejected_until_it_is_reconsidered() {
        let mut field = threaded_field();
        field
            .rejected_candidates
            .insert((idea_id("20260820T142000-000000000"), capture(4)));

        assert!(candidates(&field, &capture(4)).is_empty());

        field.rejected_candidates.clear();
        assert_eq!(candidates(&field, &capture(4)).len(), 1);
    }

    /// Recorded one way round, suppressed both ways round.
    #[test]
    fn a_rejected_pair_is_suppressed_whichever_way_it_was_recorded() {
        let mut field = field();
        field
            .rejected_pairs
            .insert(canonical(&capture(4), &capture(1)));
        field
            .rejected_pairs
            .insert(canonical(&capture(4), &capture(3)));

        assert!(candidates(&field, &capture(4)).is_empty());
    }

    #[test]
    fn returns_at_most_three_candidates_and_five_signals() {
        let mut documents: Vec<Document> = (1..=8)
            .map(|n| document(capture(n), "Dungeon seeds decide the loot and the layout."))
            .collect();
        documents.push(document(
            capture(9),
            "Dungeon seeds decide the loot and the layout.",
        ));
        let field = Field {
            corpus: Corpus::new(documents),
            ..Field::default()
        };

        let found = candidates(&field, &capture(9));

        assert_eq!(found.len(), MAX_CANDIDATES);
        for candidate in &found {
            assert!(candidate.signals.len() <= MAX_SIGNALS);
            assert!(candidate.explained() <= candidate.similarity + 1e-9);
        }
    }

    #[test]
    fn nothing_below_the_threshold_is_suggested() {
        let field = Field {
            corpus: Corpus::new([
                document(capture(1), "Dungeon seeds decide the loot."),
                document(capture(2), "The compiler passes run in order."),
            ]),
            ..Field::default()
        };

        assert!(candidates(&field, &capture(2)).is_empty());
    }

    #[test]
    fn a_capture_with_nothing_to_match_on_gets_no_candidates() {
        let field = Field {
            corpus: Corpus::new([
                document(capture(1), "Dungeon seeds decide the loot."),
                document(capture(2), "..."),
            ]),
            ..Field::default()
        };

        assert!(candidates(&field, &capture(2)).is_empty());
    }

    /// The whole corpus saying one thing is not an error, and the threshold does
    /// not save anybody from it: the smoothed idf floors a ubiquitous term at 1
    /// rather than erasing it. Three of the seven identical captures come back,
    /// which is the honest answer to "these really do all say the same thing".
    #[test]
    fn a_corpus_of_one_repeated_thought_still_answers() {
        let field = Field {
            corpus: Corpus::new((1..=7).map(|n| document(capture(n), "Seeds. Seeds. Seeds."))),
            ..Field::default()
        };

        let found = candidates(&field, &capture(1));

        assert_eq!(found.len(), MAX_CANDIDATES);
        for candidate in &found {
            near(candidate.similarity, 1.0);
        }
    }

    /// Ordering has to be total, or two runs over the same files disagree.
    #[test]
    fn ties_are_broken_by_target_kind_and_then_by_id() {
        let field = Field {
            corpus: Corpus::new([
                document(capture(1), "Dungeon seeds."),
                document(capture(2), "Dungeon seeds."),
                document(capture(3), "Dungeon seeds."),
                document(capture(4), "Dungeon seeds."),
            ]),
            threads: vec![Thread {
                id: idea_id("20260820T142000-000000000"),
                name: "Dungeon seeds".to_owned(),
                retired: false,
                members: vec![capture(1)],
            }],
            ..Field::default()
        };

        let found = candidates(&field, &capture(4));

        assert_eq!(found.len(), MAX_CANDIDATES);
        assert!(matches!(found[0].target, Target::Idea { .. }));
        assert_eq!(found[1].target.id(), capture(2).as_str());
        assert_eq!(found[2].target.id(), capture(3).as_str());
        // And asking twice gives the same answer.
        assert_eq!(found, candidates(&field, &capture(4)));
    }
}
