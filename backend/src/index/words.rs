//! The word log, folded into a table nothing else has to read a file for.
//!
//! `page_words` is derived, like everything else here. The authored copy is
//! `.rhizolog/words/`, and deleting the database costs one read of it.
//!
//! It earns its place twice. `Index::upsert` has to know what a page's total was
//! last time in order to tell a first sighting from an edit, and reading a
//! month's file to answer that on every save would be absurd. And the series has
//! to join page titles under the audience predicate, which is a join.

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, ToSql, params};

use super::audience::{Audience, VISIBLE};
use super::{Index, IndexError, bindings, from_nanos, to_nanos};
use crate::slug::Slug;
use crate::words::diff::Churn;
use crate::words::stats::Sample;
use crate::words::{Kind, Observation};

/// What indexing a page turned out to be worth recording about it.
///
/// Computed by [`Index::upsert`], because that is the one place holding both the
/// body about to be written and the body about to be replaced. Recording it is
/// somebody else's job: the log is authored data and the index does not write
/// authored data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WordChange {
    pub kind: Kind,
    pub added: u64,
    pub removed: u64,
    /// The page's count after the write. See [`Observation::total`].
    pub total: u64,
}

impl WordChange {
    /// Whether this is worth a line in the log.
    ///
    /// A baseline always is, even though it reports no writing: it is the line
    /// that stops an imported wiki being reported as written on a Tuesday, and
    /// it is what "this slug has been seen" is decided by afterwards. An
    /// observation that found nothing changed is not, which is what keeps a
    /// touched-but-unedited file out of the log.
    pub fn is_recordable(&self) -> bool {
        self.kind == Kind::Baseline || self.added > 0 || self.removed > 0
    }

    pub fn churn(&self) -> Churn {
        Churn {
            added: self.added,
            removed: self.removed,
        }
    }

    /// Read a first sighting as words just written, rather than as a wiki that
    /// was already there.
    ///
    /// [`Index::upsert`] cannot tell the two apart. A page with nothing recorded
    /// against it looks identical whether somebody just created it or the server
    /// has only now been pointed at a wiki full of them, because in both cases
    /// there is no previous body and no previous total. The difference is not in
    /// the page, it is in **who is asking**.
    ///
    /// So the startup scan takes the baseline as written, because it is
    /// reconciling with a past it did not see, and a live write does not,
    /// because a page that has just been created is words that have just
    /// arrived. Calling every new page a baseline would quietly drop the first
    /// draft of everything.
    ///
    /// The watcher counts as a live write, which has one known cost: restoring a
    /// backup or checking out a branch file by file would be reported as writing
    /// it. In practice a change on that scale arrives as a directory event and
    /// goes through the scan instead.
    pub fn as_written(self) -> Self {
        if self.kind != Kind::Baseline {
            return self;
        }

        Self {
            kind: Kind::Observed,
            added: self.total,
            removed: 0,
            total: self.total,
        }
    }
}

/// The total last recorded at a slug, since whatever last closed its series.
///
/// `None` means this slug has never been observed, or has been vacated since it
/// was: deleted, or moved away from. Either way the next thing written there is
/// a **baseline** rather than an edit of what used to be there, which is what
/// keeps a chart from showing a page losing forty thousand words and gaining
/// them back.
///
/// Ordered by `id` rather than by `at`, because two observations can share an
/// instant and the question is which line came last.
pub(super) fn last_total(connection: &Connection, slug: &str) -> rusqlite::Result<Option<u64>> {
    connection
        .query_row(
            "select total from page_words
             where slug = ?1
               and id > coalesce((
                   select max(id) from page_words
                   where (slug = ?1 and kind = 'deleted')
                      or (kind = 'moved' and src = ?1)
               ), 0)
             order by id desc
             limit 1",
            params![slug],
            |row| row.get::<_, i64>(0),
        )
        .optional()
        .map(|total| total.map(|total| total.max(0) as u64))
}

/// Decide what a write was worth, from the three things that are known about it.
///
/// The four cases are the whole of it, and the last two are the interesting
/// ones:
///
/// - **Nothing recorded at this slug.** A first sighting, so a
///   [`Kind::Baseline`]. Without it, pointing the server at an existing wiki
///   would report the whole thing as written on a Tuesday.
/// - **A previous body the log last saw.** The ordinary case, and a real
///   churn: see [`crate::words::diff::churn`].
/// - **A previous total but no previous body.** The index was deleted and the
///   file changed before the next start. The body it used to have went with
///   `pages_fts`, so the difference between the two totals is all there is, and
///   it is recorded as [`Kind::Net`] so that nobody reads it as a churn.
/// - **A previous body the log never saw.** The index is older than the log: a
///   page and the log lines describing its edits arrived together, by a
///   `git pull` or a copy from another machine, and the index still holds the
///   body from before either. Diffing against it would record writing the log
///   already holds, a second time. So it counts as no body at all, and takes
///   the path above.
///
/// Whether the log saw a body is decided by its count, which comes free with
/// the churn: `added - removed` is exactly the change in the page's count, so
/// the body's own count is `total + removed - added`, and it has to be the log's
/// last total for the churn to be news. Asking instead whether the total moved
/// would be wrong the other way, because changing one word for another is
/// writing and leaves the count where it was. What this costs is that an edit
/// to a page whose index is stale is only a net, which is exactly what a
/// deleted index already costs, and for the same reason.
///
/// The last two cases are also what makes deleting the database free, and a
/// stale one harmless. For every page nobody touched the two totals are equal,
/// so nothing is recordable and the log gains nothing. `total` is the check, and
/// this is what it checks.
pub(super) fn weigh(
    last: Option<u64>,
    before: Option<&str>,
    after: &str,
    total: u64,
) -> WordChange {
    let (kind, churn) = match (last, before) {
        (None, _) => (Kind::Baseline, Churn::default()),
        (Some(last), Some(before)) => {
            let churn = crate::words::diff::churn(before, after);

            if counted_before(churn, total) == Some(last) {
                (Kind::Observed, churn)
            } else {
                (Kind::Net, crate::words::diff::net(last, total))
            }
        }
        (Some(last), None) => (Kind::Net, crate::words::diff::net(last, total)),
    };

    WordChange {
        kind,
        added: churn.added,
        removed: churn.removed,
        total,
    }
}

/// The count a previous body had, recovered from the churn rather than counted
/// a second time over what may be a very long chapter. `None` only if the churn
/// and the total disagree about the page, which
/// [`crate::words::diff::churn`] promises they never do.
fn counted_before(churn: Churn, total: u64) -> Option<u64> {
    total.checked_add(churn.removed)?.checked_sub(churn.added)
}

fn insert(connection: &Connection, observation: &Observation) -> Result<(), IndexError> {
    connection.execute(
        "insert into page_words (at, slug, actor, account, kind, added, removed, total, src)
         values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
        params![
            to_nanos(observation.at, "at")?,
            observation.slug.as_str(),
            &observation.actor,
            observation
                .account
                .as_ref()
                .map(|account| account.as_str())
                .unwrap_or(""),
            observation.kind.as_str(),
            observation.added as i64,
            observation.removed as i64,
            observation.total as i64,
            observation.from.as_ref().map(Slug::as_str),
        ],
    )?;

    Ok(())
}

impl Index {
    /// Fold one freshly written observation in.
    pub async fn record_words(&self, observation: &Observation) -> Result<(), IndexError> {
        let observation = observation.clone();
        self.with_connection(move |connection| insert(connection, &observation))
            .await
    }

    /// Replace the whole table from the log.
    ///
    /// Wholesale rather than file by file, and the reason is that a partial
    /// rebuild has states a whole one cannot get into. The log is a few hundred
    /// kilobytes a year, this runs at startup and behind `POST /api/reindex`,
    /// and it is the operation that makes "deleting the database loses nothing"
    /// true of the word series as well as of everything else.
    pub async fn rebuild_words(&self, observations: &[Observation]) -> Result<(), IndexError> {
        let observations = observations.to_vec();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute("delete from page_words", [])?;

            for observation in &observations {
                insert(&transaction, observation)?;
            }

            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// The total last recorded at a slug. See [`last_total`].
    ///
    /// For the callers that have to decide something before writing rather than
    /// while writing: whether a page nobody ever observed is worth a `deleted`
    /// marker, say.
    pub async fn last_word_total(&self, slug: &Slug) -> Result<Option<u64>, IndexError> {
        let slug = slug.to_string();

        self.with_connection(move |connection| Ok(last_total(connection, &slug)?))
            .await
    }

    /// Every observation in `[from, to)`, reduced to what the series needs.
    ///
    /// The bucketing happens in Rust, for the reason [`crate::words::stats`]
    /// gives: which local day an instant falls in is the caller's question, not
    /// the database's.
    ///
    /// Page titles come along because the series ranks pages, and looking each
    /// one up afterwards would be a query per page. **The title is what the
    /// audience filters**, not the row: the slug is the observation's own
    /// content, and hiding somebody's working history from them would be the
    /// wrong reading of a rule that exists to protect other people's pages. A
    /// page this caller may not read is ranked under its slug, which is the
    /// label an unwritten page already gets.
    pub async fn word_samples(
        &self,
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        audience: &Audience,
    ) -> Result<Vec<Sample>, IndexError> {
        let from = to_nanos(from, "from")?;
        let to = to_nanos(to, "to")?;
        let audience = audience.clone();

        self.with_connection(move |connection| {
            let visible = audience.params();
            let mut query = connection.prepare(&format!(
                "select page_words.at, page_words.slug, pages.title, page_words.actor,
                        page_words.kind, page_words.added, page_words.removed
                 from page_words
                 left join pages on pages.slug = page_words.slug and {VISIBLE}
                 where page_words.at >= :from and page_words.at < :to
                 order by page_words.at, page_words.id",
            ))?;

            let own: [(&'static str, &dyn ToSql); 2] = [(":from", &from), (":to", &to)];
            let rows = query.query_map(bindings(&own, &visible).as_slice(), |row| {
                Ok((
                    from_nanos(row.get::<_, i64>(0)?),
                    row.get::<_, String>(1)?,
                    row.get::<_, Option<String>>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, i64>(5)?,
                    row.get::<_, i64>(6)?,
                ))
            })?;

            let mut samples = Vec::new();

            for row in rows {
                let (at, slug, title, actor, kind, added, removed) = row?;
                // A row whose slug or kind no longer parses cannot be attributed
                // to anything, and the log is append-only so it cannot be fixed
                // in place. Leaving it out is the honest answer.
                let (Ok(slug), Some(kind)) = (Slug::parse(&slug), Kind::parse(&kind)) else {
                    continue;
                };

                samples.push(Sample {
                    at,
                    slug,
                    title,
                    actor,
                    kind,
                    added: added.max(0) as u64,
                    removed: removed.max(0) as u64,
                });
            }

            Ok(samples)
        })
        .await
    }

    /// How many observations the whole log holds, for a caller that wants to
    /// know whether there is a series to draw at all.
    pub async fn count_words_observed(&self) -> Result<usize, IndexError> {
        self.with_connection(|connection| {
            let count: i64 =
                connection.query_row("select count(*) from page_words", [], |row| row.get(0))?;
            Ok(count.max(0) as usize)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::users::Username;

    fn slug(raw: &str) -> Slug {
        Slug::parse(raw).expect("valid slug")
    }

    fn at(text: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(text)
            .expect("valid timestamp")
            .with_timezone(&Utc)
    }

    fn observation(when: &str, page: &str, kind: Kind, total: u64) -> Observation {
        Observation {
            at: at(when),
            slug: slug(page),
            actor: "web".to_owned(),
            account: None,
            kind,
            added: 0,
            removed: 0,
            total,
            from: None,
        }
    }

    async fn index() -> Index {
        Index::open(None).await.expect("open index")
    }

    /// Nothing recorded and nothing to diff against: a wiki that was already
    /// there when the server first looked at it.
    #[test]
    fn a_page_nobody_has_a_record_of_is_a_baseline() {
        let change = weigh(None, None, "One two three.\n", 3);

        assert_eq!(change.kind, Kind::Baseline);
        assert_eq!((change.added, change.removed, change.total), (0, 0, 3));
        assert!(
            change.is_recordable(),
            "a baseline reports no writing and still has to be written down"
        );
    }

    /// The same facts, read by somebody who knows the page has just been made.
    #[test]
    fn a_live_write_reads_a_first_sighting_as_words_that_just_arrived() {
        let written = weigh(None, None, "One two three.\n", 3).as_written();

        assert_eq!(written.kind, Kind::Observed);
        assert_eq!((written.added, written.removed, written.total), (3, 0, 3));

        // And it does nothing to anything else, so a caller can apply it without
        // having to know which case it is in.
        let ordinary = weigh(Some(3), Some("One two three.\n"), "One two.\n", 2);
        assert_eq!(ordinary.as_written(), ordinary);
    }

    #[test]
    fn a_previous_body_makes_it_a_churn() {
        let change = weigh(
            Some(5),
            Some("The ferry was very late.\n"),
            "The ferry was early.\n",
            4,
        );

        assert_eq!(change.kind, Kind::Observed);
        assert_eq!((change.added, change.removed), (1, 2));
        assert_eq!(change.total, 4);
    }

    /// The index was deleted and the file changed before the next start. The
    /// body it used to have went with `pages_fts`, so the difference between the
    /// two totals is all there is.
    #[test]
    fn a_total_with_no_body_to_compare_is_a_net() {
        let change = weigh(Some(100), None, "irrelevant\n", 140);

        assert_eq!(change.kind, Kind::Net);
        assert_eq!((change.added, change.removed), (40, 0));
    }

    /// The property that makes deleting the database free: on a rebuild every
    /// page takes the `net` path, and for every page nobody touched the two
    /// totals agree, so there is nothing to record.
    #[test]
    fn a_rebuild_over_an_untouched_wiki_records_nothing() {
        let change = weigh(Some(41230), None, "irrelevant\n", 41230);

        assert!(change.churn().is_nothing());
        assert!(!change.is_recordable());
    }

    /// Changing one word for another is writing, and leaves the count where it
    /// was. The reason a stale index is recognised by the previous body's count
    /// rather than by whether the total moved.
    #[test]
    fn an_edit_that_keeps_the_count_is_still_a_churn() {
        let change = weigh(Some(3), Some("One two three.\n"), "One two four.\n", 3);

        assert_eq!(change.kind, Kind::Observed);
        assert_eq!((change.added, change.removed), (1, 1));
    }

    /// The index holds a body the log has moved past: a page and the line
    /// describing its edit arrived together, and the index was built before
    /// either. Diffing that body would record the edit a second time.
    #[test]
    fn a_body_the_log_never_saw_is_not_a_churn() {
        let caught_up = weigh(
            Some(5),
            Some("One two three.\n"),
            "One two three four five.\n",
            5,
        );

        assert_eq!(caught_up.kind, Kind::Net);
        assert!(
            !caught_up.is_recordable(),
            "the log already says five, and so does the page"
        );

        // And if the page has moved on from the log as well, that much is
        // recorded, as the net it is.
        let further = weigh(
            Some(5),
            Some("One two three.\n"),
            "One two three four five six seven.\n",
            7,
        );

        assert_eq!(further.kind, Kind::Net);
        assert_eq!((further.added, further.removed), (2, 0));
        assert_eq!(further.total, 7);
    }

    #[tokio::test]
    async fn the_last_total_is_the_last_line_at_that_slug() {
        let index = index().await;

        for (when, total) in [("2026-08-01T00:00:00Z", 100), ("2026-08-02T00:00:00Z", 140)] {
            index
                .record_words(&observation(when, "a", Kind::Observed, total))
                .await
                .expect("record");
        }

        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(140));
        assert_eq!(index.last_word_total(&slug("b")).await.unwrap(), None);
    }

    /// The reason the marker exists: without it the two pages would be one
    /// series, and the chart would show a page losing forty thousand words and
    /// gaining them back.
    #[tokio::test]
    async fn a_delete_closes_the_series_at_that_slug() {
        let index = index().await;

        index
            .record_words(&observation(
                "2026-08-01T00:00:00Z",
                "a",
                Kind::Observed,
                400,
            ))
            .await
            .expect("record");
        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(400));

        index
            .record_words(&observation("2026-08-02T00:00:00Z", "a", Kind::Deleted, 0))
            .await
            .expect("record");
        assert_eq!(
            index.last_word_total(&slug("a")).await.unwrap(),
            None,
            "a page written here next is a fresh page, not an edit of the old one"
        );
    }

    /// A move closes the slug it left as well as opening the one it arrived at.
    #[tokio::test]
    async fn a_move_vacates_the_slug_it_came_from() {
        let index = index().await;

        index
            .record_words(&observation(
                "2026-08-01T00:00:00Z",
                "a",
                Kind::Observed,
                400,
            ))
            .await
            .expect("record");
        index
            .record_words(&Observation {
                from: Some(slug("a")),
                ..observation("2026-08-02T00:00:00Z", "b", Kind::Moved, 400)
            })
            .await
            .expect("record");

        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), None);
        assert_eq!(
            index.last_word_total(&slug("b")).await.unwrap(),
            Some(400),
            "and the series continues where the page went"
        );
    }

    /// A split names a second slug the way a move does and vacates neither,
    /// which is the whole difference between the two markers. Getting it wrong
    /// would break a chapter's series in half every time somebody cut one, and
    /// the next edit to the page that was split would come back as a baseline.
    #[tokio::test]
    async fn a_split_leaves_the_page_it_cut_and_opens_the_one_it_made() {
        let index = index().await;

        index
            .record_words(&observation(
                "2026-08-01T00:00:00Z",
                "a",
                Kind::Observed,
                400,
            ))
            .await
            .expect("record");
        index
            .record_words(&observation("2026-08-02T00:00:00Z", "a", Kind::Split, 240))
            .await
            .expect("record");
        index
            .record_words(&Observation {
                from: Some(slug("a")),
                ..observation("2026-08-02T00:00:00Z", "b", Kind::Split, 160)
            })
            .await
            .expect("record");

        assert_eq!(
            index.last_word_total(&slug("a")).await.unwrap(),
            Some(240),
            "the page that was split is still that page"
        );
        assert_eq!(index.last_word_total(&slug("b")).await.unwrap(), Some(160));
    }

    /// The page that grew carries on from its new total; the page that was
    /// folded in is closed by the delete that removed it, not by the merge.
    #[tokio::test]
    async fn a_merge_carries_one_series_on_and_closes_the_other() {
        let index = index().await;

        for (page, kind, total) in [
            ("a", Kind::Observed, 400),
            ("b", Kind::Observed, 160),
            ("a", Kind::Merged, 560),
        ] {
            index
                .record_words(&Observation {
                    from: (kind == Kind::Merged).then(|| slug("b")),
                    ..observation("2026-08-02T00:00:00Z", page, kind, total)
                })
                .await
                .expect("record");
        }

        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(560));
        assert_eq!(
            index.last_word_total(&slug("b")).await.unwrap(),
            Some(160),
            "the merge alone says nothing about the slug the words came from"
        );

        index
            .record_words(&observation("2026-08-02T00:00:01Z", "b", Kind::Deleted, 0))
            .await
            .expect("record");
        assert_eq!(index.last_word_total(&slug("b")).await.unwrap(), None);
    }

    /// Writing at a closed slug opens a new series, and the one after that is an
    /// ordinary edit again.
    #[tokio::test]
    async fn a_series_reopens_after_it_was_closed() {
        let index = index().await;

        for (when, kind, total) in [
            ("2026-08-01T00:00:00Z", Kind::Observed, 400),
            ("2026-08-02T00:00:00Z", Kind::Deleted, 0),
            ("2026-08-03T00:00:00Z", Kind::Baseline, 12),
            ("2026-08-04T00:00:00Z", Kind::Observed, 30),
        ] {
            index
                .record_words(&observation(when, "a", kind, total))
                .await
                .expect("record");
        }

        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(30));
    }

    #[tokio::test]
    async fn a_rebuild_replaces_the_table_rather_than_adding_to_it() {
        let index = index().await;
        let log = [
            observation("2026-08-01T00:00:00Z", "a", Kind::Baseline, 100),
            Observation {
                added: 40,
                account: Username::parse("tim").ok(),
                ..observation("2026-08-02T00:00:00Z", "a", Kind::Observed, 140)
            },
        ];

        index.rebuild_words(&log).await.expect("rebuild");
        assert_eq!(index.count_words_observed().await.unwrap(), 2);

        // Twice over, because a rebuild runs on every start.
        index.rebuild_words(&log).await.expect("rebuild");
        assert_eq!(index.count_words_observed().await.unwrap(), 2);
        assert_eq!(index.last_word_total(&slug("a")).await.unwrap(), Some(140));
    }

    #[tokio::test]
    async fn samples_come_back_in_order_and_only_from_the_window() {
        let index = index().await;

        for when in [
            "2026-07-31T23:00:00Z",
            "2026-08-01T09:00:00Z",
            "2026-08-01T17:00:00Z",
            "2026-08-02T00:00:00Z",
        ] {
            index
                .record_words(&Observation {
                    added: 10,
                    ..observation(when, "a", Kind::Observed, 10)
                })
                .await
                .expect("record");
        }

        let samples = index
            .word_samples(
                at("2026-08-01T00:00:00Z"),
                at("2026-08-02T00:00:00Z"),
                &Audience::Everything,
            )
            .await
            .expect("samples");

        assert_eq!(samples.len(), 2);
        assert_eq!(samples[0].at, at("2026-08-01T09:00:00Z"));
        assert_eq!(samples[1].at, at("2026-08-01T17:00:00Z"));
        assert_eq!(samples[0].actor, "web");
    }

    /// An observation against a page nobody has written keeps its slug and has
    /// no title, which is the same answer a page the caller may not read gets.
    #[tokio::test]
    async fn a_sample_for_a_page_that_is_not_indexed_has_no_title() {
        let index = index().await;
        index
            .record_words(&observation(
                "2026-08-01T09:00:00Z",
                "gone",
                Kind::Observed,
                0,
            ))
            .await
            .expect("record");

        let samples = index
            .word_samples(
                at("2026-08-01T00:00:00Z"),
                at("2026-08-02T00:00:00Z"),
                &Audience::Everything,
            )
            .await
            .expect("samples");

        assert_eq!(samples[0].slug, slug("gone"));
        assert_eq!(samples[0].title, None);
    }
}
