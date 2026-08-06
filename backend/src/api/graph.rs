//! The link graph, tags, and meta-stats.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, State};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::api::AppState;
use crate::api::pages::parse_slug;
use crate::error::AppResult;
use crate::index::{self, RouteUsage};
use crate::slug::Slug;

#[derive(Debug, Serialize, ToSchema)]
pub struct OutboundLinkView {
    /// A slug for `wiki` and `internal` links, a URL for `external` ones.
    #[schema(example = "notes/rust/pinning")]
    pub target: String,
    /// The link's text, when it says something other than the target.
    #[schema(example = "why pinning exists")]
    pub display: Option<String>,
    /// `wiki` (`[[slug]]`), `internal` (a markdown link to a page), or
    /// `external`.
    #[schema(example = "wiki")]
    pub kind: String,
    /// Whether this points at a page that exists. A `false` here is not an
    /// error — it is a wanted page.
    pub resolved: bool,
    /// Title of the target page, when it exists.
    #[schema(example = "Pinning")]
    pub title: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct InboundLinkView {
    /// The page that links here.
    pub slug: Slug,
    /// Its title.
    #[schema(example = "Async in Rust")]
    pub title: String,
    /// What the link says, when that differs from the target.
    #[schema(example = "the async notes")]
    pub display: Option<String>,
    /// `wiki` or `internal`. A page cannot be reached by an external link.
    #[schema(example = "wiki")]
    pub kind: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TimeRefView {
    pub id: crate::times::TimeId,
    /// The activity this time was tracked under.
    #[schema(example = "Deep work")]
    pub name: String,
    pub start: DateTime<Utc>,
    /// `null` while the timer is running.
    pub end: Option<DateTime<Utc>>,
    #[schema(example = 4470)]
    pub seconds: u64,
}

/// The time tracked against a page.
///
/// A summary and a sample, not a list. A page you actually work on collects a
/// time entry every time you start a timer, so hundreds is ordinary — which is
/// exactly why these are not `inbound` links. Mixed into the backlinks they
/// would bury them; reported here they are one line with a total on it.
#[derive(Debug, Serialize, ToSchema)]
pub struct PageTimesView {
    /// Entries attached to this page.
    #[schema(example = 143)]
    pub entries: usize,
    /// Total tracked, with running entries counted up to now.
    #[schema(example = 97920)]
    pub seconds: u64,
    /// How many of them are running right now.
    #[schema(example = 1)]
    pub running: usize,
    /// Distinct activity names tracked against this page.
    #[schema(example = 4)]
    pub groups: usize,
    /// The most recent few, capped. `GET /api/times?page={slug}` has the rest.
    pub recent: Vec<TimeRefView>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PageLinksResponse {
    /// The slug that was asked about.
    pub slug: Slug,
    /// Whether the page itself exists. Links can point at pages that do not.
    pub exists: bool,
    /// Links this page makes, in document order, deduplicated per kind.
    pub outbound: Vec<OutboundLinkView>,
    /// Pages that link here — the backlinks.
    pub inbound: Vec<InboundLinkView>,
    /// Time tracked against this page.
    ///
    /// A different kind of edge, and deliberately not one of the `inbound`
    /// links: a page you work on collects one of these every time a timer
    /// starts, so hundreds is ordinary and mixing them in would bury the
    /// backlinks. Summarised here instead, with `GET /api/times?page={slug}`
    /// for the full list.
    pub times: PageTimesView,
}

/// Both directions of a page's links.
///
/// The slug does not have to name a page that exists: asking about a wanted
/// page returns what already points at it, which is what you want to see before
/// deciding whether to write it.
///
/// Both directions come back together because that is how they are used — a
/// page view shows its links and its backlinks at once, and one round trip
/// beats two.
#[utoipa::path(
    get,
    path = "/api/links/{*slug}",
    tag = "graph",
    params(("slug" = String, Path, description = "Page slug", example = "notes/rust/async")),
    responses(
        (status = 200, description = "Links into and out of the page", body = PageLinksResponse),
        (status = 400, description = "The slug is not valid", body = crate::error::ErrorResponse),
    ),
)]
pub async fn links(
    State(state): State<AppState>,
    Path(raw): Path<String>,
) -> AppResult<Json<PageLinksResponse>> {
    let slug = parse_slug(&raw)?;
    let links = state.index.links_for(&slug).await?;
    let exists = state.store.exists(&slug).await?;
    let times = state.index.page_times(&slug, Utc::now()).await?;

    Ok(Json(PageLinksResponse {
        slug,
        exists,
        times: PageTimesView {
            entries: times.entries,
            seconds: times.seconds,
            running: times.running,
            groups: times.groups,
            recent: times
                .recent
                .into_iter()
                .map(|entry| TimeRefView {
                    id: entry.id,
                    name: entry.name,
                    start: entry.start,
                    end: entry.end,
                    seconds: entry.seconds,
                })
                .collect(),
        },
        outbound: links
            .outbound
            .into_iter()
            .map(|link| OutboundLinkView {
                resolved: link.is_resolved(),
                target: link.target,
                display: link.display,
                kind: link.kind.as_str().to_owned(),
                title: link.title,
            })
            .collect(),
        inbound: links
            .inbound
            .into_iter()
            .map(|link| InboundLinkView {
                slug: link.slug,
                title: link.title,
                display: link.display,
                kind: link.kind.as_str().to_owned(),
            })
            .collect(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagCountView {
    /// The tag, exactly as pages spell it. Tags are not normalised.
    #[schema(example = "rust")]
    pub tag: String,
    /// How many pages carry this tag.
    #[schema(example = 4)]
    pub pages: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct TagsResponse {
    /// Most-used first.
    pub tags: Vec<TagCountView>,
}

/// Every tag in the wiki, with page counts.
#[utoipa::path(
    get,
    path = "/api/tags",
    tag = "graph",
    responses((status = 200, description = "All tags, most-used first", body = TagsResponse)),
)]
pub async fn tags(State(state): State<AppState>) -> AppResult<Json<TagsResponse>> {
    let tags = state.index.tags().await?;

    Ok(Json(TagsResponse {
        tags: tags
            .into_iter()
            .map(|count| TagCountView {
                tag: count.tag,
                pages: count.pages,
            })
            .collect(),
    }))
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkTotalsView {
    /// Links pointing at pages, whether or not those pages exist.
    #[schema(example = 9)]
    pub internal: usize,
    /// Links leaving the wiki.
    #[schema(example = 2)]
    pub external: usize,
    /// Internal links whose target exists.
    #[schema(example = 8)]
    pub resolved: usize,
    /// Internal links whose target has not been written yet.
    #[schema(example = 1)]
    pub wanted: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct WantedPageView {
    /// The slug that is linked to but does not exist.
    #[schema(example = "notes/rust/streams")]
    pub slug: String,
    /// How many pages link to it — how badly it is wanted.
    #[schema(example = 2)]
    pub referrers: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct LinkedPageView {
    pub slug: Slug,
    /// The page's effective title.
    #[schema(example = "Async in Rust")]
    pub title: String,
    /// How many pages link to it.
    #[schema(example = 3)]
    pub referrers: usize,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PageRefView {
    pub slug: Slug,
    /// The page's effective title.
    #[schema(example = "Async in Rust")]
    pub title: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct RouteUsageView {
    /// The route template, as the OpenAPI document spells it.
    #[schema(example = "/api/pages/{slug}")]
    pub route: String,
    /// The HTTP method, uppercase.
    #[schema(example = "GET")]
    pub method: String,
    /// Calls since the wiki was created. Counts survive restarts; they live in
    /// the durable half of the index, not the rebuildable half.
    #[schema(example = 412)]
    pub count: u64,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct StatsResponse {
    /// Pages in the index.
    #[schema(example = 6)]
    pub pages: usize,
    /// Distinct tags across every page.
    #[schema(example = 4)]
    pub tags: usize,
    /// Link counts, split by where they point.
    pub links: LinkTotalsView,
    /// Pages nothing links to.
    #[schema(example = 1)]
    pub orphan_count: usize,
    /// A sample of them, capped.
    pub orphans: Vec<PageRefView>,
    /// Distinct slugs that are linked to but do not exist.
    #[schema(example = 1)]
    pub wanted_count: usize,
    /// The most-referenced of them, capped.
    pub wanted: Vec<WantedPageView>,
    /// The most-linked-to pages, capped.
    pub most_linked: Vec<LinkedPageView>,
    /// Every tag, most-used first.
    pub tag_counts: Vec<TagCountView>,
    /// When the index was last reconciled with the wiki directory.
    pub last_indexed: Option<DateTime<Utc>>,
    /// API calls per route since the wiki was created, busiest first.
    pub api_usage: Vec<RouteUsageView>,
}

/// The wiki's meta-stats.
///
/// Orphans and wanted pages are the two most useful numbers here, and they are
/// two sides of the same thing: pages nothing reaches, and reaches with nothing
/// at the end. A wiki that branches chaotically accumulates both.
#[utoipa::path(
    get,
    path = "/api/stats",
    tag = "graph",
    responses((status = 200, description = "Meta-stats for the whole wiki", body = StatsResponse)),
)]
pub async fn stats(State(state): State<AppState>) -> AppResult<Json<StatsResponse>> {
    let stats = state.index.stats().await?;
    let persisted = state.index.usage().await?;

    Ok(Json(StatsResponse {
        pages: stats.pages,
        tags: stats.tags,
        links: LinkTotalsView {
            internal: stats.links.internal,
            external: stats.links.external,
            resolved: stats.links.resolved,
            wanted: stats.links.wanted,
        },
        orphan_count: stats.orphan_count,
        orphans: stats
            .orphans
            .into_iter()
            .map(|page| PageRefView {
                slug: page.slug,
                title: page.title,
            })
            .collect(),
        wanted_count: stats.wanted_count,
        wanted: stats
            .wanted
            .into_iter()
            .map(|page| WantedPageView {
                slug: page.slug,
                referrers: page.referrers,
            })
            .collect(),
        most_linked: stats
            .most_linked
            .into_iter()
            .map(|page| LinkedPageView {
                slug: page.slug,
                title: page.title,
                referrers: page.referrers,
            })
            .collect(),
        tag_counts: stats
            .tag_counts
            .into_iter()
            .map(|count| TagCountView {
                tag: count.tag,
                pages: count.pages,
            })
            .collect(),
        last_indexed: stats.last_indexed,
        api_usage: merge_usage(persisted, &state),
    }))
}

/// Persisted counts plus whatever has been tallied since the last flush, so the
/// number is current without this read having to write.
fn merge_usage(persisted: Vec<RouteUsage>, state: &AppState) -> Vec<RouteUsageView> {
    let mut totals: HashMap<(String, String), u64> = persisted
        .into_iter()
        .map(|usage| ((usage.route, usage.method), usage.count))
        .collect();

    for ((route, method), count) in state.usage.snapshot() {
        *totals.entry((route, method)).or_insert(0) += count;
    }

    let mut usage: Vec<RouteUsageView> = totals
        .into_iter()
        .map(|((route, method), count)| RouteUsageView {
            route,
            method,
            count,
        })
        .collect();

    usage.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.route.cmp(&b.route))
            .then_with(|| a.method.cmp(&b.method))
    });
    usage
}

/// Flush the in-memory tally into the index.
///
/// Anything `drain` returns is gone from memory, so a failure here loses those
/// counts. They are usage statistics; losing a minute of them on a database
/// hiccup is not worth failing a shutdown over, so this logs and moves on.
pub async fn flush_usage(index: &index::Index, usage: &crate::api::usage::UsageTally) {
    let counts = usage.drain();
    if counts.is_empty() {
        return;
    }

    let total: u64 = counts.iter().map(|(_, count)| count).sum();
    if let Err(error) = index.record_usage(counts).await {
        tracing::warn!(%error, dropped = total, "could not persist API usage counts");
    }
}
