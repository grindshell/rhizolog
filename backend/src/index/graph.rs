//! The link graph, tags, and the meta-stats the dashboard exists to show.
//!
//! Link targets are stored as written and resolved by joining against `pages`
//! at query time. Nothing caches the answer, which is what makes the graph
//! self-healing: create a page that three others already link to and those
//! three links resolve immediately, with nothing to reindex.
//!
//! It also means **unresolved links are ordinary, not broken**. A link to a
//! page nobody has written yet is a wanted page — a branch someone gestured at.
//! Together with orphans (pages nothing links to) that is the main thing
//! `/api/stats` is for.

use chrono::{DateTime, Utc};
use rusqlite::{OptionalExtension, params};

use crate::index::schema::{KEY_LAST_SYNC, TOP_N};
use crate::index::{Index, IndexError, from_nanos};
use crate::markdown::LinkKind;
use crate::slug::Slug;

/// A link leaving a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutboundLink {
    /// A slug for wiki and internal links, a URL for external ones.
    pub target: String,
    pub display: Option<String>,
    pub kind: LinkKind,
    /// Title of the page this points at, when it points at one that exists.
    pub title: Option<String>,
}

impl OutboundLink {
    /// Whether this link points at a page that exists.
    ///
    /// External links are never "resolved" — there is nothing in the wiki to
    /// resolve them against.
    pub fn is_resolved(&self) -> bool {
        self.kind.is_internal() && self.title.is_some()
    }
}

/// A link arriving at a page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InboundLink {
    pub slug: Slug,
    pub title: String,
    pub display: Option<String>,
    pub kind: LinkKind,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageLinks {
    pub outbound: Vec<OutboundLink>,
    pub inbound: Vec<InboundLink>,
}

/// A page that is linked to but does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WantedPage {
    pub slug: String,
    /// How many distinct pages link here.
    pub referrers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinkedPage {
    pub slug: Slug,
    pub title: String,
    pub referrers: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageRef {
    pub slug: Slug,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TagCount {
    pub tag: String,
    pub pages: usize,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LinkTotals {
    /// Links pointing at pages, whether or not those pages exist.
    pub internal: usize,
    pub external: usize,
    /// Internal links whose target exists.
    pub resolved: usize,
    /// Internal links whose target does not exist yet.
    pub wanted: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteUsage {
    pub route: String,
    pub method: String,
    pub count: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stats {
    pub pages: usize,
    pub tags: usize,
    pub links: LinkTotals,
    pub orphan_count: usize,
    /// The most-referenced wanted pages, capped at [`TOP_N`].
    pub wanted: Vec<WantedPage>,
    pub wanted_count: usize,
    /// Pages nothing links to, capped at [`TOP_N`].
    pub orphans: Vec<PageRef>,
    /// The most-linked-to pages, capped at [`TOP_N`].
    pub most_linked: Vec<LinkedPage>,
    /// Every tag with its page count, most-used first.
    pub tag_counts: Vec<TagCount>,
    pub last_indexed: Option<DateTime<Utc>>,
}

/// Only wiki and internal links are part of the page graph.
const IS_PAGE_LINK: &str = "links.kind != 'external'";

impl Index {
    /// Every link into and out of a page.
    ///
    /// The page itself need not exist: asking about a wanted page returns the
    /// links pointing at it, which is exactly what you want to see before
    /// deciding to write it.
    pub async fn links_for(&self, slug: &Slug) -> Result<PageLinks, IndexError> {
        let slug = slug.to_string();

        self.with_connection(move |connection| {
            let mut outbound_query = connection.prepare(&format!(
                "select links.target, links.display, links.kind, pages.title
                 from links
                 left join pages
                   on pages.slug = links.target and {IS_PAGE_LINK}
                 where links.src_slug = ?1
                 order by links.kind, links.target"
            ))?;

            let outbound = outbound_query
                .query_map(params![&slug], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<String>>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter_map(|(target, display, kind, title)| {
                    Some(OutboundLink {
                        target,
                        display,
                        kind: LinkKind::parse(&kind)?,
                        title,
                    })
                })
                .collect();

            let mut inbound_query = connection.prepare(&format!(
                "select links.src_slug, pages.title, links.display, links.kind
                 from links
                 join pages on pages.slug = links.src_slug
                 where links.target = ?1 and {IS_PAGE_LINK}
                 order by links.src_slug"
            ))?;

            let inbound = inbound_query
                .query_map(params![&slug], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, Option<String>>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter_map(|(slug, title, display, kind)| {
                    Some(InboundLink {
                        slug: Slug::parse(&slug).ok()?,
                        title,
                        display,
                        kind: LinkKind::parse(&kind)?,
                    })
                })
                .collect();

            Ok(PageLinks { outbound, inbound })
        })
        .await
    }

    /// Every tag in the wiki, most-used first.
    pub async fn tags(&self) -> Result<Vec<TagCount>, IndexError> {
        self.with_connection(|connection| {
            let mut query = connection.prepare(
                "select tag, count(*) as pages
                 from page_tags
                 group by tag
                 order by pages desc, tag asc",
            )?;

            let tags = query
                .query_map([], |row| {
                    Ok(TagCount {
                        tag: row.get(0)?,
                        pages: row.get::<_, i64>(1)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(tags)
        })
        .await
    }

    /// Everything the dashboard shows.
    pub async fn stats(&self) -> Result<Stats, IndexError> {
        self.with_connection(|connection| {
            let pages: i64 =
                connection.query_row("select count(*) from pages", [], |row| row.get(0))?;

            let (internal, external): (i64, i64) = connection.query_row(
                &format!(
                    "select
                       coalesce(sum(case when {IS_PAGE_LINK} then 1 else 0 end), 0),
                       coalesce(sum(case when links.kind = 'external' then 1 else 0 end), 0)
                     from links"
                ),
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            let resolved: i64 = connection.query_row(
                &format!(
                    "select count(*) from links
                     where {IS_PAGE_LINK}
                       and exists (select 1 from pages where pages.slug = links.target)"
                ),
                [],
                |row| row.get(0),
            )?;

            let wanted_count: i64 = connection.query_row(
                &format!(
                    "select count(distinct links.target) from links
                     where {IS_PAGE_LINK}
                       and not exists (select 1 from pages where pages.slug = links.target)"
                ),
                [],
                |row| row.get(0),
            )?;

            let orphan_count: i64 = connection.query_row(
                &format!(
                    "select count(*) from pages
                     where not exists (
                       select 1 from links
                       where links.target = pages.slug and {IS_PAGE_LINK}
                     )"
                ),
                [],
                |row| row.get(0),
            )?;

            let mut wanted_query = connection.prepare(&format!(
                "select links.target, count(distinct links.src_slug) as referrers
                 from links
                 where {IS_PAGE_LINK}
                   and not exists (select 1 from pages where pages.slug = links.target)
                 group by links.target
                 order by referrers desc, links.target asc
                 limit ?1"
            ))?;
            let wanted = wanted_query
                .query_map(params![TOP_N as i64], |row| {
                    Ok(WantedPage {
                        slug: row.get(0)?,
                        referrers: row.get::<_, i64>(1)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let mut orphan_query = connection.prepare(&format!(
                "select pages.slug, pages.title
                 from pages
                 where not exists (
                   select 1 from links
                   where links.target = pages.slug and {IS_PAGE_LINK}
                 )
                 order by pages.slug
                 limit ?1"
            ))?;
            let orphans = orphan_query
                .query_map(params![TOP_N as i64], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter_map(|(slug, title)| {
                    Some(PageRef {
                        slug: Slug::parse(&slug).ok()?,
                        title,
                    })
                })
                .collect();

            let mut linked_query = connection.prepare(&format!(
                "select links.target, pages.title, count(distinct links.src_slug) as referrers
                 from links
                 join pages on pages.slug = links.target
                 where {IS_PAGE_LINK}
                 group by links.target, pages.title
                 order by referrers desc, links.target asc
                 limit ?1"
            ))?;
            let most_linked = linked_query
                .query_map(params![TOP_N as i64], |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, i64>(2)?,
                    ))
                })?
                .collect::<Result<Vec<_>, _>>()?
                .into_iter()
                .filter_map(|(slug, title, referrers)| {
                    Some(LinkedPage {
                        slug: Slug::parse(&slug).ok()?,
                        title,
                        referrers: referrers as usize,
                    })
                })
                .collect();

            let mut tag_query = connection.prepare(
                "select tag, count(*) as pages
                 from page_tags
                 group by tag
                 order by pages desc, tag asc",
            )?;
            let tag_counts = tag_query
                .query_map([], |row| {
                    Ok(TagCount {
                        tag: row.get(0)?,
                        pages: row.get::<_, i64>(1)? as usize,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            let last_indexed: Option<String> = connection
                .query_row(
                    "select value from meta where key = ?1",
                    params![KEY_LAST_SYNC],
                    |row| row.get(0),
                )
                .optional()?;

            Ok(Stats {
                pages: pages as usize,
                tags: tag_counts.len(),
                links: LinkTotals {
                    internal: internal as usize,
                    external: external as usize,
                    resolved: resolved as usize,
                    wanted: (internal - resolved) as usize,
                },
                orphan_count: orphan_count as usize,
                wanted,
                wanted_count: wanted_count as usize,
                orphans,
                most_linked,
                tag_counts,
                last_indexed: last_indexed
                    .and_then(|value| value.parse::<i64>().ok())
                    .map(from_nanos),
            })
        })
        .await
    }

    /// Add to the persisted API usage counters.
    ///
    /// Counts arrive in batches from an in-memory tally rather than one write
    /// per request; see `api::usage`.
    pub async fn record_usage(
        &self,
        counts: Vec<((String, String), u64)>,
    ) -> Result<(), IndexError> {
        if counts.is_empty() {
            return Ok(());
        }

        self.with_connection(move |connection| {
            let transaction = connection.transaction()?;
            {
                let mut upsert = transaction.prepare(
                    "insert into api_usage (route, method, count) values (?1, ?2, ?3)
                     on conflict (route, method) do update set count = count + excluded.count",
                )?;
                for ((route, method), count) in &counts {
                    upsert.execute(params![route, method, *count as i64])?;
                }
            }
            transaction.commit()?;
            Ok(())
        })
        .await
    }

    /// Persisted API usage counts, busiest first.
    pub async fn usage(&self) -> Result<Vec<RouteUsage>, IndexError> {
        self.with_connection(|connection| {
            let mut query = connection.prepare(
                "select route, method, count from api_usage order by count desc, route asc",
            )?;

            let usage = query
                .query_map([], |row| {
                    Ok(RouteUsage {
                        route: row.get(0)?,
                        method: row.get(1)?,
                        count: row.get::<_, i64>(2)?.max(0) as u64,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;

            Ok(usage)
        })
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::page::{Frontmatter, Page};

    async fn index() -> Index {
        Index::open(None).await.expect("open in-memory index")
    }

    fn page(slug: &str, tags: &[&str], body: &str) -> Page {
        Page {
            slug: Slug::parse(slug).expect("valid slug"),
            frontmatter: Frontmatter {
                title: Some(slug.to_uppercase()),
                tags: tags.iter().map(|tag| (*tag).to_string()).collect(),
                created: None,
            },
            body: body.to_owned(),
            updated: DateTime::from_timestamp_nanos(1_700_000_000_000_000_000),
            size: body.len() as u64,
        }
    }

    async fn seed(index: &Index, slug: &str, body: &str) {
        index.upsert(&page(slug, &[], body)).await.expect("upsert");
    }

    #[tokio::test]
    async fn records_outbound_links() {
        let index = index().await;
        seed(
            &index,
            "index",
            "See [[notes/a]] and [b](notes/b.md) and [out](https://example.com).\n",
        )
        .await;
        seed(&index, "notes/a", "A page.\n").await;

        let links = index
            .links_for(&Slug::parse("index").unwrap())
            .await
            .unwrap();

        assert_eq!(links.outbound.len(), 3);
        let resolved: Vec<_> = links
            .outbound
            .iter()
            .filter(|link| link.is_resolved())
            .map(|link| link.target.as_str())
            .collect();
        assert_eq!(resolved, ["notes/a"], "only the existing page resolves");
    }

    #[tokio::test]
    async fn backlinks_are_the_other_side_of_the_same_edge() {
        let index = index().await;
        seed(&index, "notes/target", "The target.\n").await;
        seed(&index, "a", "Link to [[notes/target]].\n").await;
        seed(&index, "b", "Also [[notes/target|see this]].\n").await;

        let links = index
            .links_for(&Slug::parse("notes/target").unwrap())
            .await
            .unwrap();

        let sources: Vec<_> = links
            .inbound
            .iter()
            .map(|link| link.slug.as_str())
            .collect();
        assert_eq!(sources, ["a", "b"]);
        assert_eq!(links.inbound[1].display.as_deref(), Some("see this"));
        assert!(links.outbound.is_empty());
    }

    /// The property that makes resolution a query and not a cache.
    #[tokio::test]
    async fn creating_a_wanted_page_resolves_its_links_with_no_reindex() {
        let index = index().await;
        seed(&index, "a", "Link to [[notes/later]].\n").await;

        let before = index.stats().await.unwrap();
        assert_eq!(before.links.wanted, 1);
        assert_eq!(before.links.resolved, 0);
        assert_eq!(before.wanted[0].slug, "notes/later");

        // Only the new page is written. Nothing touches `a`.
        seed(&index, "notes/later", "Now it exists.\n").await;

        let after = index.stats().await.unwrap();
        assert_eq!(after.links.wanted, 0);
        assert_eq!(after.links.resolved, 1);
        assert!(after.wanted.is_empty());
    }

    /// ...and the reverse: deleting a page turns links to it back into wants.
    #[tokio::test]
    async fn deleting_a_page_turns_its_backlinks_into_wants() {
        let index = index().await;
        seed(&index, "a", "Link to [[notes/target]].\n").await;
        seed(&index, "notes/target", "The target.\n").await;
        assert_eq!(index.stats().await.unwrap().links.resolved, 1);

        index
            .remove(&Slug::parse("notes/target").unwrap())
            .await
            .unwrap();

        let stats = index.stats().await.unwrap();
        assert_eq!(stats.links.wanted, 1);
        assert_eq!(stats.wanted[0].slug, "notes/target");
        // `a` still has its outbound link; it just points at nothing now.
        let links = index.links_for(&Slug::parse("a").unwrap()).await.unwrap();
        assert_eq!(links.outbound.len(), 1);
        assert!(!links.outbound[0].is_resolved());
    }

    #[tokio::test]
    async fn removing_a_page_takes_its_outbound_links_with_it() {
        let index = index().await;
        seed(&index, "target", "Target.\n").await;
        seed(&index, "source", "Link to [[target]].\n").await;

        index.remove(&Slug::parse("source").unwrap()).await.unwrap();

        let links = index
            .links_for(&Slug::parse("target").unwrap())
            .await
            .unwrap();
        assert!(links.inbound.is_empty(), "a deleted page still links");
    }

    #[tokio::test]
    async fn counts_orphans_and_wanted_pages() {
        let index = index().await;
        seed(&index, "hub", "Links to [[spoke]] and [[missing]].\n").await;
        seed(&index, "spoke", "Linked to.\n").await;
        seed(&index, "lonely", "Nothing links here.\n").await;

        let stats = index.stats().await.unwrap();

        assert_eq!(stats.pages, 3);
        // `hub` and `lonely` have no inbound links.
        assert_eq!(stats.orphan_count, 2);
        let orphans: Vec<_> = stats.orphans.iter().map(|p| p.slug.as_str()).collect();
        assert_eq!(orphans, ["hub", "lonely"]);

        assert_eq!(stats.wanted_count, 1);
        assert_eq!(stats.wanted[0].slug, "missing");
    }

    #[tokio::test]
    async fn ranks_the_most_linked_pages() {
        let index = index().await;
        seed(&index, "popular", "Everyone links here.\n").await;
        seed(&index, "quiet", "Fewer do.\n").await;
        for source in ["a", "b", "c"] {
            seed(&index, source, "See [[popular]].\n").await;
        }
        seed(&index, "d", "See [[quiet]].\n").await;

        let stats = index.stats().await.unwrap();

        assert_eq!(stats.most_linked[0].slug.as_str(), "popular");
        assert_eq!(stats.most_linked[0].referrers, 3);
        assert_eq!(stats.most_linked[1].slug.as_str(), "quiet");
        assert_eq!(stats.most_linked[1].referrers, 1);
    }

    #[tokio::test]
    async fn counts_tags() {
        let index = index().await;
        index
            .upsert(&page("a", &["theory", "shared"], "body"))
            .await
            .unwrap();
        index.upsert(&page("b", &["shared"], "body")).await.unwrap();

        let tags = index.tags().await.unwrap();

        assert_eq!(tags.len(), 2);
        assert_eq!(
            tags[0],
            TagCount {
                tag: "shared".into(),
                pages: 2
            }
        );
        assert_eq!(
            tags[1],
            TagCount {
                tag: "theory".into(),
                pages: 1
            }
        );
        assert_eq!(index.stats().await.unwrap().tags, 2);
    }

    #[tokio::test]
    async fn separates_internal_and_external_link_totals() {
        let index = index().await;
        seed(
            &index,
            "a",
            "[[internal]] and [x](https://example.com) and [y](mailto:a@b.c)\n",
        )
        .await;

        let stats = index.stats().await.unwrap();

        assert_eq!(stats.links.internal, 1);
        assert_eq!(stats.links.external, 2);
    }

    #[tokio::test]
    async fn an_empty_wiki_has_empty_stats() {
        let stats = index().await.stats().await.unwrap();

        assert_eq!(stats.pages, 0);
        assert_eq!(stats.links, LinkTotals::default());
        assert!(stats.orphans.is_empty());
        assert!(stats.wanted.is_empty());
        assert!(stats.tag_counts.is_empty());
    }

    #[tokio::test]
    async fn usage_counts_accumulate() {
        let index = index().await;

        index
            .record_usage(vec![
                (("/api/pages".into(), "GET".into()), 3),
                (("/api/search".into(), "GET".into()), 1),
            ])
            .await
            .unwrap();
        index
            .record_usage(vec![(("/api/pages".into(), "GET".into()), 2)])
            .await
            .unwrap();

        let usage = index.usage().await.unwrap();

        assert_eq!(usage[0].route, "/api/pages");
        assert_eq!(usage[0].count, 5, "counts should add, not replace");
        assert_eq!(usage[1].count, 1);
    }

    /// Usage is not derived from the wiki, so unlike everything else in the
    /// index there is nowhere to rebuild it from — clearing must leave it.
    #[tokio::test]
    async fn clearing_the_index_leaves_usage_alone() {
        let index = index().await;
        seed(&index, "a", "Body.\n").await;
        index
            .record_usage(vec![(("/api/pages".into(), "GET".into()), 7)])
            .await
            .unwrap();

        index.clear().await.unwrap();

        assert_eq!(index.count().await.unwrap(), 0);
        assert_eq!(index.usage().await.unwrap()[0].count, 7);
    }
}
