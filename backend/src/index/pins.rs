//! Pinned pages: the handful a user keeps within reach.
//!
//! A pin is a page's slug and the moment it was pinned. That is all it is — the
//! title shown in the menu is looked up by joining `pages` at read time, the
//! same trick the [link graph](super::graph) uses, and for the same reason: a
//! pinned page that is renamed in its frontmatter should change its label with
//! no reindex.
//!
//! Pins live in the durable half of the schema. Nothing derives them from the
//! markdown, so a rebuild has nowhere to get them back from — see
//! [`super::schema`].
//!
//! **A pin can outlive its page.** Nothing here refuses that. Deleting a page
//! through the API takes its pin with it and moving one carries the pin along,
//! because both are deliberate acts on that page; but a file removed from
//! outside Rhizolog is indistinguishable from a move that has not finished yet,
//! and quietly forgetting a pin because a file was briefly absent would be the
//! worse failure. Such a pin comes back with `title: None`, and the UI offers to
//! remove it.
//!
//! **A pin can also outlive the caller's right to read its page**, and that has
//! deliberately the same shape: the title join carries
//! [`crate::index::audience::VISIBLE`], so a pin to a page you may not read is
//! indistinguishable from a pin to one that is gone. The slug stays, because
//! the pin *is* the slug and the list is wiki-wide — see
//! `knowledge-base/visibility.md`.

use chrono::{DateTime, Utc};
use rusqlite::params;

use crate::index::audience::{Audience, VISIBLE};
use crate::index::{Index, IndexError, from_nanos, to_nanos};
use crate::slug::Slug;

/// A pinned page, with whatever the index knows about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub slug: Slug,
    /// The page's title, or `None` when no page is indexed at that slug.
    pub title: Option<String>,
    /// When it was pinned. Re-pinning an already-pinned page does not move it.
    pub pinned_at: DateTime<Utc>,
}

impl Pin {
    /// Whether there is a page behind this pin.
    pub fn exists(&self) -> bool {
        self.title.is_some()
    }
}

impl Index {
    /// Every pin, oldest first.
    ///
    /// Oldest first rather than newest: this is a menu, and a menu whose entries
    /// reshuffle every time one is added is a menu you have to read before every
    /// click. Ties break on slug so the order is total.
    ///
    /// The join carries the visibility predicate, so a pin to a page the caller
    /// may not read comes back with no title — exactly as a pin to a deleted
    /// page does. The *slug* is not hidden and cannot be: the pin list is
    /// wiki-wide state, and the slug is what a pin is.
    pub async fn pins(&self, audience: &Audience) -> Result<Vec<Pin>, IndexError> {
        let audience = audience.clone();

        self.with_connection(move |connection| {
            let visible = audience.params();
            let mut query = connection.prepare(&format!(
                "select pins.slug, pages.title, pins.pinned_at
                 from pins
                 left join pages on pages.slug = pins.slug and {VISIBLE}
                 order by pins.pinned_at asc, pins.slug asc",
            ))?;

            let rows = query.query_map(&visible[..], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, Option<String>>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;

            let mut pins = Vec::new();
            for row in rows {
                let (slug, title, pinned_at) = row?;
                // A row whose slug no longer parses names a page that could
                // never be served, so there is nothing useful to return for it.
                let Ok(slug) = Slug::parse(&slug) else {
                    continue;
                };
                pins.push(Pin {
                    slug,
                    title,
                    pinned_at: from_nanos(pinned_at),
                });
            }
            Ok(pins)
        })
        .await
    }

    pub async fn is_pinned(&self, slug: &Slug) -> Result<bool, IndexError> {
        let slug = slug.to_string();

        self.with_connection(move |connection| {
            let count: i64 = connection.query_row(
                "select count(*) from pins where slug = ?1",
                params![&slug],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
        .await
    }

    pub async fn count_pins(&self) -> Result<usize, IndexError> {
        self.with_connection(|connection| {
            let count: i64 =
                connection.query_row("select count(*) from pins", [], |row| row.get(0))?;
            Ok(count as usize)
        })
        .await
    }

    /// Pin a page. Pinning one that is already pinned changes nothing.
    ///
    /// `insert or ignore` rather than `insert or replace`: re-pinning must not
    /// reset `pinned_at`, or a double click would move the entry to the end of
    /// the menu.
    pub async fn pin(&self, slug: &Slug, at: DateTime<Utc>) -> Result<(), IndexError> {
        let slug = slug.to_string();
        let at = to_nanos(at, "pinned at")?;

        self.with_connection(move |connection| {
            connection.execute(
                "insert or ignore into pins (slug, pinned_at) values (?1, ?2)",
                params![&slug, at],
            )?;
            Ok(())
        })
        .await
    }

    /// Remove a pin, reporting whether there was one.
    ///
    /// The bool is what lets `DELETE /api/pins/{slug}` answer 404 for a page
    /// that was never pinned, rather than a 204 that did nothing.
    pub async fn unpin(&self, slug: &Slug) -> Result<bool, IndexError> {
        let slug = slug.to_string();

        self.with_connection(move |connection| {
            let removed = connection.execute("delete from pins where slug = ?1", params![&slug])?;
            Ok(removed > 0)
        })
        .await
    }

    /// Carry a pin across a move, keeping the moment it was pinned.
    ///
    /// A no-op when the page was not pinned. If the destination is somehow
    /// already pinned the older pin wins, which keeps the menu's order stable.
    pub async fn repin(&self, from: &Slug, to: &Slug) -> Result<(), IndexError> {
        let from = from.to_string();
        let to = to.to_string();

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;
            transaction.execute(
                "insert or ignore into pins (slug, pinned_at)
                 select ?2, pinned_at from pins where slug = ?1",
                params![&from, &to],
            )?;
            transaction.execute("delete from pins where slug = ?1", params![&from])?;
            transaction.commit()?;
            Ok(())
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::page::{Frontmatter, Page};

    /// Every case here runs against a wiki with no accounts. What a pin to a
    /// page the caller cannot read looks like is `tests/visibility.rs`.
    const EVERYONE: Audience = Audience::Everything;

    async fn index() -> Index {
        Index::open(None).await.expect("open in-memory index")
    }

    fn at(nanos: i64) -> DateTime<Utc> {
        DateTime::from_timestamp_nanos(nanos)
    }

    fn slug(raw: &str) -> Slug {
        Slug::parse(raw).expect("valid slug")
    }

    async fn seed(index: &Index, raw: &str, title: &str) {
        index
            .upsert(&Page {
                slug: slug(raw),
                frontmatter: Frontmatter {
                    title: Some(title.to_owned()),
                    tags: Vec::new(),
                    created: None,
                    ..Frontmatter::default()
                },
                body: "Body.\n".to_owned(),
                updated: at(1_700_000_000_000_000_000),
                size: 6,
            })
            .await
            .expect("upsert");
    }

    #[tokio::test]
    async fn pins_round_trip_with_the_page_title() {
        let index = index().await;
        seed(&index, "notes/quick", "Quick Notes").await;

        index.pin(&slug("notes/quick"), at(1)).await.unwrap();
        let pins = index.pins(&EVERYONE).await.unwrap();

        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].slug.as_str(), "notes/quick");
        assert_eq!(pins[0].title.as_deref(), Some("Quick Notes"));
        assert!(pins[0].exists());
        assert!(index.is_pinned(&slug("notes/quick")).await.unwrap());
    }

    /// The menu must not reshuffle. Re-pinning is idempotent in both senses:
    /// no duplicate row, and no new position.
    #[tokio::test]
    async fn re_pinning_keeps_the_original_position() {
        let index = index().await;
        seed(&index, "a", "A").await;
        seed(&index, "b", "B").await;

        index.pin(&slug("a"), at(1)).await.unwrap();
        index.pin(&slug("b"), at(2)).await.unwrap();
        index.pin(&slug("a"), at(3)).await.unwrap();

        let pins = index.pins(&EVERYONE).await.unwrap();
        let slugs: Vec<&str> = pins.iter().map(|pin| pin.slug.as_str()).collect();
        assert_eq!(slugs, ["a", "b"]);
    }

    #[tokio::test]
    async fn unpinning_reports_whether_there_was_a_pin() {
        let index = index().await;
        seed(&index, "a", "A").await;
        index.pin(&slug("a"), at(1)).await.unwrap();

        assert!(index.unpin(&slug("a")).await.unwrap());
        assert!(!index.unpin(&slug("a")).await.unwrap());
        assert!(index.pins(&EVERYONE).await.unwrap().is_empty());
    }

    /// A pin whose page is gone is reported as missing, not dropped — the UI
    /// needs something to offer to remove.
    #[tokio::test]
    async fn a_pin_survives_its_page_and_is_reported_as_missing() {
        let index = index().await;
        seed(&index, "notes/gone", "Gone").await;
        index.pin(&slug("notes/gone"), at(1)).await.unwrap();

        index.remove(&slug("notes/gone")).await.unwrap();
        let pins = index.pins(&EVERYONE).await.unwrap();

        assert_eq!(pins.len(), 1);
        assert_eq!(pins[0].title, None);
        assert!(!pins[0].exists());
    }

    #[tokio::test]
    async fn a_pin_follows_a_move_without_losing_its_place() {
        let index = index().await;
        seed(&index, "first", "First").await;
        seed(&index, "notes/old", "Old").await;
        index.pin(&slug("first"), at(1)).await.unwrap();
        index.pin(&slug("notes/old"), at(2)).await.unwrap();

        seed(&index, "archive/new", "New").await;
        index.remove(&slug("notes/old")).await.unwrap();
        index
            .repin(&slug("notes/old"), &slug("archive/new"))
            .await
            .unwrap();

        let pins = index.pins(&EVERYONE).await.unwrap();
        let slugs: Vec<&str> = pins.iter().map(|pin| pin.slug.as_str()).collect();
        // Still second, not bumped to the front by a fresh timestamp.
        assert_eq!(slugs, ["first", "archive/new"]);
        assert_eq!(pins[1].title.as_deref(), Some("New"));
    }

    #[tokio::test]
    async fn repinning_a_page_that_was_never_pinned_does_nothing() {
        let index = index().await;
        seed(&index, "a", "A").await;

        index.repin(&slug("a"), &slug("b")).await.unwrap();

        assert!(index.pins(&EVERYONE).await.unwrap().is_empty());
    }

    /// Pins are not derived from the wiki, so a rebuild has nowhere to get them
    /// back from — clearing the index must leave them alone.
    #[tokio::test]
    async fn clearing_the_index_leaves_pins_alone() {
        let index = index().await;
        seed(&index, "notes/quick", "Quick Notes").await;
        index.pin(&slug("notes/quick"), at(1)).await.unwrap();

        index.clear().await.unwrap();

        assert_eq!(index.count_pins().await.unwrap(), 1);
        // The page is gone from the index until the next scan, so the title is
        // unresolved — but the pin itself survived.
        assert_eq!(index.pins(&EVERYONE).await.unwrap()[0].title, None);
    }
}
