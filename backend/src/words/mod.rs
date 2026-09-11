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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

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
    /// There was no previous body to trust, so `added` and `removed` are a net
    /// change split by its sign rather than a churn.
    ///
    /// The one place a net figure appears, and it is labelled as one. It happens
    /// when the index was deleted and pages changed before the next start, when
    /// the index holds a body the log has since moved past, and when the log
    /// missed a line the index did not: the log knows the last total, the body
    /// in `pages_fts` is not the one that total was counted from, and the
    /// difference between the two totals is all there is to record.
    Net,
    /// The page arrived here from another slug. Carries both.
    ///
    /// A marker rather than a churn: nothing was written. It closes the series
    /// at the old slug and continues it at the new one.
    Moved,
    /// The page was cut in two, or is the half that was cut off. The second
    /// carries the slug it came from.
    ///
    /// A marker for the same reason [`Kind::Moved`] is one: moving the boundary
    /// between two pages writes nothing and unwrites nothing. Recorded as an
    /// ordinary observation instead, a split would report a chapter losing two
    /// thousand words and another gaining them on a day nobody wrote a sentence,
    /// which is exactly the reading this log exists to refuse.
    ///
    /// Unlike a move it closes **no** series. The page that was split is still
    /// that page and its history runs straight through; the half that was cut off
    /// begins one.
    Split,
    /// Another page's words were folded into this one. Carries the slug they
    /// came from, which is gone.
    ///
    /// The other half of [`Kind::Split`] and a marker on the same terms. The page
    /// that was merged away gets its own [`Kind::Deleted`] line, which is what
    /// closes its series.
    Merged,
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
            Self::Split => "split",
            Self::Merged => "merged",
            Self::Deleted => "deleted",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        match raw {
            "baseline" => Some(Self::Baseline),
            "observed" => Some(Self::Observed),
            "net" => Some(Self::Net),
            "moved" => Some(Self::Moved),
            "split" => Some(Self::Split),
            "merged" => Some(Self::Merged),
            "deleted" => Some(Self::Deleted),
            _ => None,
        }
    }

    /// Whether this line reports writing rather than bookkeeping.
    pub fn is_work(self) -> bool {
        matches!(self, Self::Observed | Self::Net)
    }

    /// Whether this line ends whatever series came before it at a slug.
    ///
    /// Neither [`Kind::Split`] nor [`Kind::Merged`] does. Both name a second slug
    /// the way a move does and neither vacates one: a page that was split is
    /// still there, and a page that grew by a merge was already there. The page
    /// that was merged away is closed by its own [`Kind::Deleted`] line.
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
    /// Serialises this process's use of the log: every append, and every read
    /// that `page_words` is rebuilt from.
    ///
    /// An `O_APPEND` write of one short line is atomic on every filesystem worth
    /// naming, but there is no reason to depend on that when one lock removes
    /// the question. It is why a process must hold **one** `WordLog` rather than
    /// opening a second over the same directory.
    ///
    /// It has to cover more than the write, which is what [`WordLog::hold`] is
    /// for. A line and its row in `page_words` are written one after the other,
    /// and a rebuild of that table which read the log before the line and
    /// replaced the table after the row would drop the row; one which read after
    /// the line and replaced before the row would leave it there twice. So
    /// [`record`] holds the log across both, and so does the rebuild.
    ///
    /// And it carries what this process last knew the log's files to be, so a
    /// caller holding it can ask whether anybody else has written since. See
    /// [`Held::unchanged`].
    gate: std::sync::Arc<Mutex<Seen>>,
}

/// A month file's length and when it was last written, which is what "has
/// anybody touched this" is decided by.
type FileStamp = (u64, SystemTime);

/// What this process last knew each month file to be. `None` until the log is
/// first read, and after anything that leaves the answer in doubt; both mean
/// the next check reads the log.
type Seen = Option<BTreeMap<PathBuf, FileStamp>>;

impl WordLog {
    /// Open (creating if necessary) the word log for the wiki at `wiki_root`.
    pub async fn open(wiki_root: impl AsRef<Path>) -> Result<Self, WordLogError> {
        let root = wiki_root.as_ref().join(INTERNAL_DIR).join(WORDS_DIR);
        tokio::fs::create_dir_all(&root).await?;

        Ok(Self {
            root,
            gate: std::sync::Arc::new(Mutex::new(None)),
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

    /// Hold the log still until the guard is dropped: nothing in this process
    /// appends to it, or rebuilds `page_words` from it, in the meantime.
    ///
    /// For a caller whose two steps have to be one, which is [`record`] and the
    /// rebuild in `index::sync`. The note on the lock says why.
    pub async fn hold(&self) -> Held<'_> {
        Held {
            log: self,
            seen: self.gate.lock().await,
        }
    }

    /// Append one observation.
    pub async fn append(&self, observation: &Observation) -> Result<(), WordLogError> {
        self.hold().await.append(observation).await
    }

    /// Every observation in the log, oldest first. See [`Held::read`].
    pub async fn read(&self) -> Result<(Vec<Observation>, usize), WordLogError> {
        self.hold().await.read().await
    }
}

/// The log, held still. See [`WordLog::hold`].
pub struct Held<'a> {
    log: &'a WordLog,
    seen: tokio::sync::MutexGuard<'a, Seen>,
}

impl Held<'_> {
    /// Whether the log is still exactly what this process last read or wrote:
    /// the same month files, each the same length and last written at the same
    /// instant.
    ///
    /// `false` whenever it cannot be sure, because a wrong `false` costs a read
    /// and a wrong `true` costs a pulled edit counted twice.
    pub async fn unchanged(&self) -> bool {
        match (self.seen.as_ref(), stamps(&self.log.root).await) {
            (Some(seen), Ok(now)) => *seen == now,
            _ => false,
        }
    }

    /// Append one observation.
    pub async fn append(&mut self, observation: &Observation) -> Result<(), WordLogError> {
        let line = format!("{}\n", observation.line());
        let path = self.log.month(observation.at);

        // Whether the file is still what this process last saw, asked before
        // writing. If somebody else has written to it since, stamping it after
        // this line would claim their lines had been read, so the whole record is
        // dropped instead and the next check reads the log. A write landing in
        // the instant between this and the line below is the one thing it can
        // miss.
        let known = match (self.seen.as_ref(), stamp(&path).await) {
            (Some(seen), Ok(now)) => seen.get(&path).copied() == now,
            _ => false,
        };

        let mut file = tokio::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&path)
            .await?;

        file.write_all(line.as_bytes()).await?;
        // Not `sync_all`: this is one line of bookkeeping written on every save,
        // and paying for a flush to the platter each time would make saving a
        // page slower for a record whose worst case is one lost observation that
        // the next write to the page reports as a `net`.
        file.flush().await?;
        drop(file);

        match (known, stamp(&path).await) {
            (true, Ok(Some(after))) => {
                if let Some(seen) = self.seen.as_mut() {
                    seen.insert(path, after);
                }
            }
            _ => *self.seen = None,
        }

        Ok(())
    }

    /// Every observation in the log, oldest first.
    ///
    /// The whole thing, because it is small: a line is under a hundred bytes and
    /// a busy year is a few hundred kilobytes. This is what `page_words` is
    /// rebuilt from, at startup and by the watcher whenever somebody else has
    /// written to it, so reading it has to be the cheap and obviously correct
    /// operation rather than the clever one.
    ///
    /// Lines that will not parse are skipped and counted, not fatal. A log is
    /// appended to by a running server; a truncated last line after a hard
    /// power-off should cost that line and nothing else.
    pub async fn read(&mut self) -> Result<(Vec<Observation>, usize), WordLogError> {
        // Stamped before anything is read, so a line landing while this reads is
        // a change the next time anybody asks, rather than recorded as read.
        // In order of name, which for `YYYY-MM.log` is by date.
        let stamps = stamps(&self.log.root).await?;

        let mut observations = Vec::new();
        let mut skipped = 0;

        for month in stamps.keys() {
            let text = tokio::fs::read_to_string(month).await?;

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
        *self.seen = Some(stamps);

        Ok((observations, skipped))
    }
}

/// Every month file's stamp, as the directory holds them now.
async fn stamps(root: &Path) -> std::io::Result<BTreeMap<PathBuf, FileStamp>> {
    let mut stamps = BTreeMap::new();
    let mut entries = tokio::fs::read_dir(root).await?;

    while let Some(entry) = entries.next_entry().await? {
        let path = entry.path();
        if path.extension().is_none_or(|extension| extension != "log") {
            continue;
        }
        if let Some(stamp) = stamp(&path).await? {
            stamps.insert(path, stamp);
        }
    }

    Ok(stamps)
}

/// One file's stamp, or `None` if it is not there.
///
/// Asked of the file rather than read off the directory listing, which on NTFS
/// is brought up to date lazily and can lag behind a file just written.
async fn stamp(path: &Path) -> std::io::Result<Option<FileStamp>> {
    match tokio::fs::metadata(path).await {
        Ok(metadata) => Ok(Some((metadata.len(), metadata.modified()?))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
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
/// next write to the page, from anywhere, finds the index's body disagreeing
/// with the last total the log has and reports the difference as a
/// [`Kind::Net`]. The words are not lost, only the day and the tool that wrote
/// them, and the churn of whichever edit happens to come next.
///
/// Not the next startup scan, which is what this used to say. A scan skips a
/// page whose file still matches the index, and after a write that got as far
/// as the index it does.
pub async fn record(log: &WordLog, index: &Index, observation: Observation) {
    // Held across both halves, so that a rebuild of `page_words` from the log
    // cannot land between the line and its row. See the lock on `WordLog`.
    let mut held = log.hold().await;

    if let Err(error) = held.append(&observation).await {
        tracing::error!(
            slug = %observation.slug,
            %error,
            "could not write to the word log; the next write to this page will report the difference as a net"
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

/// One side of a split: which slug, and what it came to.
///
/// A pair rather than four loose arguments, because the two halves take the same
/// two values and a call site that got them crossed would write each page's total
/// against the other's slug.
#[derive(Debug, Clone, Copy)]
pub struct Half<'a> {
    pub slug: &'a Slug,
    /// The page's own count after the cut. See [`Observation::total`].
    pub total: u64,
}

/// Record that a page was cut in two.
///
/// Two lines, one at each slug, both with `added` and `removed` at zero. The
/// words on either side of the cut are the words that were there a moment ago,
/// so a split is a boundary moving rather than a day's work: recorded as
/// ordinary observations the two lines would say that somebody unwrote half a
/// chapter and wrote another one, on a day they moved a cursor.
///
/// What the lines are for is [`Observation::total`], which every slug's series
/// is checked against. Without them the next startup scan would find both files
/// disagreeing with the log and report the difference as a [`Kind::Net`], which
/// is the same wrong number with a worse label on it.
pub async fn split(
    log: &WordLog,
    index: &Index,
    head: Half<'_>,
    tail: Half<'_>,
    by: &By,
    at: DateTime<Utc>,
) {
    let line = |half: Half<'_>, from| Observation {
        at,
        slug: half.slug.clone(),
        actor: by.actor.clone(),
        account: by.account.clone(),
        kind: Kind::Split,
        added: 0,
        removed: 0,
        total: half.total,
        from,
    };

    record(log, index, line(head, None)).await;
    // The tail names where it came from, exactly as a move does, so a reader
    // following one page's history back can cross the cut.
    record(log, index, line(tail, Some(head.slug.clone()))).await;
}

/// Record that one page's words were folded into another.
///
/// One line, at the page that grew, naming the page that is gone. A marker on
/// [`split`]'s terms and for its reason: the words arrived from somewhere else
/// and were not written today.
///
/// The page they came from is closed by the [`deleted`] marker its removal
/// writes, which is the ordinary one and is not this function's to write.
pub async fn merged(
    log: &WordLog,
    index: &Index,
    from: &Slug,
    into: &Slug,
    by: &By,
    at: DateTime<Utc>,
    total: u64,
) {
    record(
        log,
        index,
        Observation {
            at,
            slug: into.clone(),
            actor: by.actor.clone(),
            account: by.account.clone(),
            kind: Kind::Merged,
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

    /// The half cut off a page names where it came from, which is what lets a
    /// reader follow one chapter's history across the cut.
    #[test]
    fn a_split_carries_the_slug_it_was_cut_from() {
        let written = Observation {
            kind: Kind::Split,
            from: Some(slug("book/one/opening")),
            added: 0,
            removed: 0,
            ..observation()
        };

        assert_eq!(Observation::parse(&written.line()), Some(written));
    }

    #[test]
    fn a_merge_carries_the_slug_the_words_came_from() {
        let written = Observation {
            kind: Kind::Merged,
            from: Some(slug("book/one/opening")),
            added: 0,
            removed: 0,
            ..observation()
        };

        assert_eq!(Observation::parse(&written.line()), Some(written));
    }

    /// Neither is a day's work and neither vacates a slug. Getting the second
    /// wrong would break a page's series in half every time it was split.
    #[test]
    fn neither_marker_is_work_and_neither_closes_a_series() {
        for kind in [Kind::Split, Kind::Merged] {
            assert!(
                !kind.is_work(),
                "{} counted as words written",
                kind.as_str()
            );
            assert!(
                !kind.closes_a_series(),
                "{} vacated the slug it was written at",
                kind.as_str()
            );
        }
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

    /// The log knows whether anybody else has written to it since this process
    /// last looked, and errs towards saying they have.
    #[tokio::test]
    async fn the_log_notices_a_write_that_was_not_its_own() {
        let directory = tempfile::tempdir().expect("temp dir");
        let log = WordLog::open(directory.path()).await.expect("open");
        let elsewhere = WordLog::open(directory.path())
            .await
            .expect("another writer");

        assert!(
            !log.hold().await.unchanged().await,
            "never read, so it cannot know"
        );

        log.read().await.expect("read");
        assert!(log.hold().await.unchanged().await);

        log.append(&observation()).await.expect("append");
        assert!(
            log.hold().await.unchanged().await,
            "its own line is not news"
        );

        elsewhere
            .append(&observation())
            .await
            .expect("append elsewhere");
        assert!(!log.hold().await.unchanged().await, "somebody else's is");

        // And a line of its own on top does not paper over theirs.
        log.append(&observation()).await.expect("append");
        assert!(!log.hold().await.unchanged().await);

        log.read().await.expect("read");
        assert!(log.hold().await.unchanged().await, "until it has read them");
    }
}
