//! The word log: what was written, when, and by which tool.
//!
//! The hours have a heat map. The words get the same thing beside them, on the
//! same terms: a chart, not a streak, and nothing that congratulates you.
//!
//! Two decisions shape everything here, and both are argued in
//! `knowledge-base/long-form.md`:
//!
//! - **It records churn, not net change.** An assistant rewriting two thousand
//!   words into nineteen hundred is not "minus one hundred". A signed total is
//!   the metric this feature exists to replace, so an observation carries
//!   `added` and `removed` and leaves the subtraction to whoever wants it.
//! - **It is authored data, not an index.** A writing history cannot be
//!   reconstructed from anything, so it lives in files under `.rhizolog/words/`
//!   and not in `index.db`, which every document in this project tells the
//!   reader is safe to delete. That makes it the **fourth** authored tree,
//!   beside `times/`, `ideas/` and `users/`, and unlike the last of those it is
//!   not secret: back it up, commit it.
//!
//! A line is never edited and never removed. A page's life leaves further
//! records rather than rewriting earlier ones, which is what lets a chapter
//! renamed halfway through a book keep one continuous series.

pub mod diff;
pub mod stats;

use std::path::{Path, PathBuf};

use chrono::{DateTime, SecondsFormat, Utc};
use thiserror::Error;
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;

use crate::index::{Index, WordChange};
use crate::slug::Slug;
use crate::store::INTERNAL_DIR;
use crate::users::Username;

/// Where the log lives inside the wiki, under [`crate::store::INTERNAL_DIR`].
pub const WORDS_DIR: &str = "words";

/// The header a caller labels its writes with.
///
/// **A label is a claim, not a proof.** On an open wiki anything that can write
/// can claim anything, which is fine, because the question it answers is
/// bookkeeping about your own tools rather than security. On a wiki with
/// accounts the observation records the label *and* the account, in two separate
/// fields, so a label can never be used to claim another account: the account
/// comes from the session and is not something a header can set.
pub const ACTOR_HEADER: &str = "x-rhizolog-actor";

/// An API write that named no tool.
pub const ACTOR_API: &str = "api";
/// An edit the watcher noticed: the writer in their own editor.
pub const ACTOR_FILE: &str = "file";
/// The startup scan, or `POST /api/reindex`.
pub const ACTOR_SCAN: &str = "scan";
/// What the dashboard sends.
pub const ACTOR_WEB: &str = "web";

/// The longest label that will be accepted.
pub const MAX_ACTOR: usize = 64;

/// Read a label off the wire.
///
/// `None` for anything that cannot go in the log: a label is written into a
/// tab-separated line, so a tab or a newline in one would produce a record that
/// reads back as something else. Refusing is better than trimming, because
/// silently rewriting somebody's provenance is worse than telling them their
/// header was no good.
pub fn actor(raw: &str) -> Option<String> {
    let label = raw.trim();

    let usable =
        !label.is_empty() && label.len() <= MAX_ACTOR && !label.chars().any(char::is_control);

    usable.then(|| label.to_owned())
}

/// Who a write is recorded as.
///
/// Two fields rather than one, because they answer different questions: which
/// tool made the write, and which person it was made as. The account comes from
/// the session and is not something a header can set, so a label can never be
/// used to claim somebody else's.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct By {
    pub actor: String,
    /// Absent on a wiki with no accounts, which has no name to give.
    pub account: Option<Username>,
}

impl By {
    /// A tool acting on nobody's behalf: the watcher, the startup scan.
    pub fn tool(actor: &str) -> Self {
        Self {
            actor: actor.to_owned(),
            account: None,
        }
    }

    /// An edit the watcher noticed, which is the writer in their own editor.
    pub fn file() -> Self {
        Self::tool(ACTOR_FILE)
    }

    /// The startup scan, or `POST /api/reindex`.
    pub fn scan() -> Self {
        Self::tool(ACTOR_SCAN)
    }
}

/// What one line of the log is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// The first time this slug was ever seen. `added` and `removed` are zero
    /// and `total` is what was already there.
    ///
    /// Without it, importing an existing wiki would report the whole thing as
    /// written on a Tuesday.
    Baseline,
    /// A diff against the body that was there before. The ordinary case.
    Observed,
    /// The previous body was not available, so `added` and `removed` are a net
    /// change split by its sign rather than a churn.
    ///
    /// The one place a net figure appears, and it is labelled as one. It happens
    /// when the index was deleted and pages changed before the next start: the
    /// log knows the last total, `pages_fts` no longer holds the last body, and
    /// the difference between the two is all there is to record.
    Net,
    /// The page arrived here from another slug. Carries both.
    ///
    /// A marker rather than a churn: nothing was written. It closes the series
    /// at the old slug and continues it at the new one.
    Moved,
    /// The page is gone.
    ///
    /// `removed` is **zero**, deliberately. The words were written and deleting
    /// the file does not unwrite them, so a delete is not a day on which you
    /// unwrote four thousand words. What the marker is for is starting a fresh
    /// series if something is later written at the same slug.
    Deleted,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Baseline => "baseline",
            Self::Observed => "observed",
            Self::Net => "net",
            Self::Moved => "moved",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "baseline" => Some(Self::Baseline),
            "observed" => Some(Self::Observed),
            "net" => Some(Self::Net),
            "moved" => Some(Self::Moved),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }

    /// Whether this line reports writing rather than bookkeeping.
    pub fn is_work(self) -> bool {
        matches!(self, Self::Observed | Self::Net)
    }

    /// Whether this line ends whatever series came before it at a slug.
    pub fn closes_a_series(self) -> bool {
        matches!(self, Self::Deleted | Self::Moved)
    }
}

/// One line of the log.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observation {
    pub at: DateTime<Utc>,
    pub slug: Slug,
    /// Which tool made the write. See [`ACTOR_HEADER`].
    pub actor: String,
    /// Which person it was made as. Absent on a wiki with no accounts, which is
    /// the same thing `owner` does on a capture.
    pub account: Option<Username>,
    pub kind: Kind,
    pub added: u64,
    pub removed: u64,
    /// The page's own count after this observation.
    ///
    /// **This is the check.** A line can be verified against the page rather
    /// than believed, and a missed observation shows up as a discontinuity
    /// instead of quietly skewing the series. It is also what lets a scan after
    /// `rm index.db` tell "nothing changed" from "something did".
    pub total: u64,
    /// Where the page came from, for [`Kind::Moved`] and nothing else.
    pub from: Option<Slug>,
}

impl Observation {
    /// The line as it is written, without its newline.
    ///
    /// Tab-separated, because a slug may contain a space and may never contain a
    /// control character. That is exactly what makes a tab a safe delimiter and
    /// a space an unsafe one.
    fn line(&self) -> String {
        let account = self.account.as_ref().map(Username::as_str).unwrap_or("");
        let from = self.from.as_ref().map_or("", Slug::as_str);

        format!(
            "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
            self.at.to_rfc3339_opts(SecondsFormat::Nanos, true),
            self.slug,
            self.actor,
            account,
            self.kind.as_str(),
            self.added,
            self.removed,
            self.total,
            from,
        )
    }

    /// Read one line back.
    ///
    /// A line with **more** than nine fields is accepted and its extras ignored,
    /// so a log written by a later version stays readable by this one. A line
    /// with fewer, or with a field that will not parse, is not: it is skipped by
    /// the caller rather than failing the whole month, because one bad line
    /// should not cost a year of history.
    fn parse(line: &str) -> Option<Self> {
        let fields: Vec<&str> = line.split('\t').collect();
        let [
            at,
            slug,
            actor,
            account,
            kind,
            added,
            removed,
            total,
            from,
            ..,
        ] = fields.as_slice()
        else {
            return None;
        };

        Some(Self {
            at: DateTime::parse_from_rfc3339(at).ok()?.with_timezone(&Utc),
            slug: Slug::parse(slug).ok()?,
            actor: self::actor(actor)?,
            account: (!account.is_empty())
                .then(|| Username::parse(account).ok())
                .flatten(),
            kind: Kind::parse(kind)?,
            added: added.parse().ok()?,
            removed: removed.parse().ok()?,
            total: total.parse().ok()?,
            from: (!from.is_empty()).then(|| Slug::parse(from).ok()).flatten(),
        })
    }
}

#[derive(Debug, Error)]
pub enum WordLogError {
    #[error("the word log could not be read or written")]
    Io(#[from] std::io::Error),
}

/// The append-only log under `.rhizolog/words/`.
///
/// **One file per month, one line per observation**, which departs from the
/// one-file-per-record shape `times/` and `ideas/` use. The reason is frequency:
/// a time entry is a document somebody may open and correct, and a word
/// observation is a machine's reading, never edited, arriving every time a file
/// is saved. A file per save would be thousands of files a month, and the
/// `YYYY-MM` directory that makes the time log survivable would not save it.
#[derive(Debug, Clone)]
pub struct WordLog {
    root: PathBuf,
    /// Serialises appends within this process.
    ///
    /// An `O_APPEND` write of one short line is atomic on every filesystem worth
    /// naming, but there is no reason to depend on that when one lock removes
    /// the question. It is why a process must hold **one** `WordLog` rather than
    /// opening a second over the same directory.
    gate: std::sync::Arc<Mutex<()>>,
}

impl WordLog {
    /// Open (creating if necessary) the word log for the wiki at `wiki_root`.
    pub async fn open(wiki_root: impl AsRef<Path>) -> Result<Self, WordLogError> {
        let root = wiki_root.as_ref().join(INTERNAL_DIR).join(WORDS_DIR);
        tokio::fs::create_dir_all(&root).await?;

        Ok(Self {
            root,
            gate: std::sync::Arc::new(Mutex::new(())),
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn root_display(&self) -> String {
        crate::store::display_path(&self.root)
    }

    /// The file an instant belongs in, `YYYY-MM.log` in UTC.
    ///
    /// UTC rather than the writer's local month, because the file is storage and
    /// an instant is an instant. Which local day a line falls on is decided at
    /// read time from an offset, which is the same answer `/api/time-stats`
    /// gives and for the same reason.
    fn month(&self, at: DateTime<Utc>) -> PathBuf {
        self.root.join(format!("{}.log", at.format("%Y-%m")))
    }

    /// Append one observation.
    pub async fn append(&self, observation: &Observation) -> Result<(), WordLogError> {
        let line = format!("{}\n", observation.line());
        let path = self.month(observation.at);

        let _held = self.gate.lock().await;
        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .await?;

        file.write_all(line.as_bytes()).await?;
        // Not `sync_all`: this is one line of bookkeeping written on every save,
        // and paying for a flush to the platter each time would make saving a
        // page slower for a record whose worst case is one lost observation that
        // the next startup scan reports as a `net`.
        file.flush().await?;

        Ok(())
    }

    /// Every observation in the log, oldest first.
    ///
    /// The whole thing, because it is small: a line is under a hundred bytes and
    /// a busy year is a few hundred kilobytes. This is what `page_words` is
    /// rebuilt from, so reading it has to be the cheap and obviously correct
    /// operation rather than the clever one.
    ///
    /// Lines that will not parse are skipped and counted, not fatal. A log is
    /// appended to by a running server; a truncated last line after a hard
    /// power-off should cost that line and nothing else.
    pub async fn read(&self) -> Result<(Vec<Observation>, usize), WordLogError> {
        let mut months: Vec<PathBuf> = Vec::new();
        let mut entries = tokio::fs::read_dir(&self.root).await?;

        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            if path.extension().is_some_and(|extension| extension == "log") {
                months.push(path);
            }
        }

        // By name, which for `YYYY-MM.log` is by date.
        months.sort();

        let mut observations = Vec::new();
        let mut skipped = 0;

        for month in months {
            let text = tokio::fs::read_to_string(&month).await?;

            for line in text.lines() {
                if line.trim().is_empty() {
                    continue;
                }
                match Observation::parse(line) {
                    Some(observation) => observations.push(observation),
                    None => skipped += 1,
                }
            }
        }

        // Within a month the file is already in order, and the files were sorted
        // by month. This puts a log whose clock went backwards in order anyway,
        // which costs one sort of a small vector and removes a way for the
        // series to come out scrambled.
        observations.sort_by_key(|observation| observation.at);

        Ok((observations, skipped))
    }
}

// ------------------------------------------------------- writing one down

/// Write an observation to the log and fold it into the index.
///
/// The file first, then the table over it, which is the order every authored
/// write in this project uses and for the same reason: the files are the truth
/// and the index is a reading of them.
///
/// **A failure here never fails the write it describes.** By the time this runs
/// the page is on disk and in the index; the observation is bookkeeping about
/// it, and answering a successful write with a 500 would make a caller retry and
/// write the page twice. So it is logged loudly and swallowed.
///
/// What that costs is one line, and it degrades rather than disappearing: the
/// next startup scan compares the file against the last total the log has and
/// reports the difference as a [`Kind::Net`]. The words are not lost, only the
/// day and the tool that wrote them.
pub async fn record(log: &WordLog, index: &Index, observation: Observation) {
    if let Err(error) = log.append(&observation).await {
        tracing::error!(
            slug = %observation.slug,
            %error,
            "could not write to the word log; the next scan will report the difference as a net"
        );
        return;
    }

    if let Err(error) = index.record_words(&observation).await {
        // The authored half is safely on disk, so this costs a stale series
        // until the next reindex rather than anything permanent.
        tracing::error!(
            slug = %observation.slug,
            %error,
            "wrote to the word log and could not index it; POST /api/reindex will catch up"
        );
    }
}

/// Record what indexing a page turned out to be worth, if it was worth anything.
pub async fn observe(
    log: &WordLog,
    index: &Index,
    slug: &Slug,
    by: &By,
    at: DateTime<Utc>,
    change: WordChange,
) {
    if !change.is_recordable() {
        return;
    }

    record(
        log,
        index,
        Observation {
            at,
            slug: slug.clone(),
            actor: by.actor.clone(),
            account: by.account.clone(),
            kind: change.kind,
            added: change.added,
            removed: change.removed,
            total: change.total,
            from: None,
        },
    )
    .await;
}

/// Record that a page arrived at `to` from `from`.
///
/// History is never rewritten, so this is a new line rather than an edit of the
/// old slug's. A reader follows the chain, which is what lets a chapter renamed
/// halfway through a book keep one continuous series.
pub async fn moved(
    log: &WordLog,
    index: &Index,
    from: &Slug,
    to: &Slug,
    by: &By,
    at: DateTime<Utc>,
    total: u64,
) {
    record(
        log,
        index,
        Observation {
            at,
            slug: to.clone(),
            actor: by.actor.clone(),
            account: by.account.clone(),
            kind: Kind::Moved,
            added: 0,
            removed: 0,
            total,
            from: Some(from.clone()),
        },
    )
    .await;
}

/// Record that a page is gone.
///
/// The history outlives it. What the marker is for is the next page written at
/// the same slug, which starts a series of its own rather than looking like a
/// forty-thousand-word deletion followed by a forty-thousand-word day.
pub async fn deleted(log: &WordLog, index: &Index, slug: &Slug, by: &By, at: DateTime<Utc>) {
    record(
        log,
        index,
        Observation {
            at,
            slug: slug.clone(),
            actor: by.actor.clone(),
            account: by.account.clone(),
            kind: Kind::Deleted,
            added: 0,
            removed: 0,
            total: 0,
            from: None,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slug(raw: &str) -> Slug {
        Slug::parse(raw).expect("valid slug")
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn observation() -> Observation {
        Observation {
            at: at("2026-08-25T14:25:30.123456789Z"),
            slug: slug("book/one/the ferry"),
            actor: "claude-code".to_owned(),
            account: Some(Username::parse("tim").expect("valid username")),
            kind: Kind::Observed,
            added: 1900,
            removed: 2000,
            total: 41230,
            from: None,
        }
    }

    /// The format the plan writes down, field for field.
    #[test]
    fn a_line_is_nine_tab_separated_fields() {
        let line = observation().line();

        assert_eq!(
            line,
            "2026-08-25T14:25:30.123456789Z\tbook/one/the ferry\tclaude-code\ttim\tobserved\t\
             1900\t2000\t41230\t"
        );
        assert_eq!(line.split('\t').count(), 9);
    }

    /// A slug may hold a space and may never hold a control character, which is
    /// the whole reason the delimiter is a tab.
    #[test]
    fn a_slug_with_a_space_in_it_round_trips() {
        let written = observation();
        assert_eq!(Observation::parse(&written.line()), Some(written));
    }

    #[test]
    fn an_open_wiki_writes_an_empty_account_and_reads_it_back_as_none() {
        let written = Observation {
            account: None,
            ..observation()
        };

        assert!(written.line().contains("\t\tobserved\t"));
        assert_eq!(Observation::parse(&written.line()), Some(written));
    }

    #[test]
    fn a_move_carries_both_slugs() {
        let written = Observation {
            kind: Kind::Moved,
            from: Some(slug("book/one/opening")),
            added: 0,
            removed: 0,
            ..observation()
        };

        assert_eq!(Observation::parse(&written.line()), Some(written));
    }

    /// A log written by a later version has to stay readable by this one, and an
    /// append-only file cannot be migrated.
    #[test]
    fn a_line_with_extra_fields_keeps_the_nine_it_understands() {
        let line = format!("{}\tsomething-new", observation().line());
        assert_eq!(Observation::parse(&line), Some(observation()));
    }

    #[test]
    fn a_line_that_will_not_parse_is_none_rather_than_a_guess() {
        assert_eq!(Observation::parse(""), None);
        assert_eq!(Observation::parse("not\tenough\tfields"), None);
        // A slug that no longer validates, a kind nobody wrote, a count that is
        // not a number.
        let bad = observation().line().replace("observed", "pondered");
        assert_eq!(Observation::parse(&bad), None);
        let bad = observation().line().replace("41230", "lots");
        assert_eq!(Observation::parse(&bad), None);
    }

    #[test]
    fn a_label_that_could_not_be_written_is_refused_rather_than_trimmed() {
        assert_eq!(actor("claude-code"), Some("claude-code".to_owned()));
        assert_eq!(actor("  web  "), Some("web".to_owned()));
        assert_eq!(actor(""), None);
        assert_eq!(actor("   "), None);
        assert_eq!(actor("two\tfields"), None);
        assert_eq!(actor("a\nline"), None);
        assert_eq!(actor(&"x".repeat(MAX_ACTOR + 1)), None);
    }

    #[tokio::test]
    async fn appends_land_in_the_month_they_belong_to_and_read_back_in_order() {
        let directory = tempfile::tempdir().expect("temp dir");
        let log = WordLog::open(directory.path()).await.expect("open");

        let july = Observation {
            at: at("2026-07-31T23:00:00Z"),
            ..observation()
        };
        let august = observation();

        // Written out of order on purpose.
        log.append(&august).await.expect("append");
        log.append(&july).await.expect("append");

        assert!(log.root().join("2026-07.log").exists());
        assert!(log.root().join("2026-08.log").exists());

        let (read, skipped) = log.read().await.expect("read");
        assert_eq!(skipped, 0);
        assert_eq!(read, vec![july, august]);
    }

    /// A truncated last line after a hard power-off should cost that line and
    /// nothing else.
    #[tokio::test]
    async fn a_broken_line_costs_itself_and_not_the_month() {
        let directory = tempfile::tempdir().expect("temp dir");
        let log = WordLog::open(directory.path()).await.expect("open");

        log.append(&observation()).await.expect("append");
        tokio::fs::write(
            log.root().join("2026-09.log"),
            "half a line with no fields\n",
        )
        .await
        .expect("write");

        let (read, skipped) = log.read().await.expect("read");
        assert_eq!(read, vec![observation()]);
        assert_eq!(skipped, 1);
    }

    #[tokio::test]
    async fn a_log_nobody_has_written_to_is_empty_rather_than_missing() {
        let directory = tempfile::tempdir().expect("temp dir");
        let log = WordLog::open(directory.path()).await.expect("open");

        assert_eq!(log.read().await.expect("read"), (Vec::new(), 0));
    }
}
