//! The link graph, tags, and meta-stats.

use std::collections::HashMap;

use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::api::AppState;
use crate::api::pages::parse_slug;
use crate::auth::Viewer;
use crate::error::AppResult;
use crate::index::{self, GraphOptions, RouteUsage};
use crate::slug::Slug;

/// How many pages one graph carries when nobody says otherwise.
///
/// Enough that an ordinary wiki arrives whole, and small enough that the first
/// call from a large one still draws something readable rather than a hairball.
const DEFAULT_NODES: usize = 400;
const MAX_NODES: usize = 2000;

const DEFAULT_DEPTH: usize = 2;
/// Past this a walk has usually crossed the whole wiki anyway, and the query
/// that finds out is the expensive one.
const MAX_DEPTH: usize = 6;

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
    viewer: Viewer,
    Path(raw): Path<String>,
) -> AppResult<Json<PageLinksResponse>> {
    let slug = parse_slug(&raw)?;
    let audience = viewer.audience();
    let links = state.index.links_for(&slug, &audience).await?;
    // Whether a page **you can read** is there. A page you cannot read has to
    // report `exists: false`, or this endpoint answers the one question a
    // private page's slug was hiding: is there something here.
    let exists = state.index.is_visible(&slug, &audience).await?;
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

#[derive(Debug, Default, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct GraphQuery {
    /// Only pages carrying this tag.
    #[param(example = "rust")]
    pub tag: Option<String>,
    /// Only pages at or under this slug path. Hierarchical and stops at the
    /// separator, exactly as on `GET /api/pages`.
    #[param(example = "notes/rust")]
    pub prefix: Option<String>,
    /// Walk outward from this page rather than drawing the whole wiki.
    ///
    /// The walk follows links in **both** directions, because a page's
    /// neighbourhood is what it points at and what points at it. The slug need
    /// not name a page that exists: a wanted page's neighbourhood is the set of
    /// pages waiting on it.
    #[param(example = "notes/rust/async")]
    pub root: Option<String>,
    /// How many hops out the walk goes. Defaults to 2, capped at 6, and means
    /// nothing without a `root`.
    #[param(example = 2, minimum = 0, maximum = 6)]
    pub depth: Option<usize>,
    /// Whether pages that are linked to but not written are nodes. Defaults to
    /// true — they are the branches the wiki has gestured at, and usually the
    /// most interesting thing in the picture.
    pub wanted: Option<bool>,
    /// How many **pages** the view may carry, capped at 2000. Wanted pages hang
    /// off the survivors and are not counted against it.
    ///
    /// When it bites, the best-connected pages survive: a graph cut down to its
    /// least connected pages is a scatter of dots that says nothing.
    #[param(example = 400)]
    pub limit: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GraphNodeView {
    #[schema(example = "notes/rust/async")]
    pub slug: String,
    /// The page's title, or the slug itself when nothing has been written
    /// there.
    #[schema(example = "Async in Rust")]
    pub title: String,
    /// `false` for a wanted page. Not an error — see `/api/stats`.
    pub exists: bool,
    /// Distinct pages linking here, across the **whole wiki** rather than this
    /// view. A hub therefore still reads as one inside a filter, and the gap
    /// between this and the edges actually returned says the branch reaches
    /// outside what was asked for.
    #[schema(example = 3)]
    pub inbound: usize,
    /// Distinct pages this one links to, likewise wiki-wide.
    #[schema(example = 2)]
    pub outbound: usize,
    /// Empty for a wanted page, which has no frontmatter to carry any.
    #[schema(example = json!(["rust", "async"]))]
    pub tags: Vec<String>,
    /// Hops from `root`, or `null` when the query had none.
    #[schema(example = 1)]
    pub distance: Option<usize>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GraphEdgeView {
    #[schema(example = "notes/rust/async")]
    pub source: String,
    #[schema(example = "notes/rust/pinning")]
    pub target: String,
    /// `wiki`, `internal`, or both when the same page is linked twice over.
    /// One line to draw either way.
    #[schema(example = json!(["wiki"]))]
    pub kinds: Vec<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct GraphResponse {
    /// Sorted by slug, so the same wiki always arrives in the same order.
    pub nodes: Vec<GraphNodeView>,
    /// Directed. A mutual pair is two edges, because which way a link points is
    /// most of what the graph has to say.
    pub edges: Vec<GraphEdgeView>,
    /// Pages that matched the filters, before `limit` was applied.
    #[schema(example = 6)]
    pub matched: usize,
    /// Whether `limit` dropped any of them.
    pub truncated: bool,
    /// The root the walk started from, echoed back.
    pub root: Option<Slug>,
    /// The depth that was applied, after clamping. `null` without a root.
    #[schema(example = 2)]
    pub depth: Option<usize>,
    /// The limit that was applied, after clamping.
    #[schema(example = 400)]
    pub limit: usize,
}

/// The link graph, as something you can draw.
///
/// `/api/links/{slug}` answers "where does this page sit"; this answers "what
/// shape is the wiki". Nodes are pages — **including ones nobody has written**,
/// which are the branches the wiki has gestured at and the reason the picture is
/// worth looking at.
///
/// Three rules decide what comes back:
///
/// 1. `tag` and `prefix` select pages, and `root` narrows to a neighbourhood.
///    They intersect.
/// 2. An edge is returned when both of its ends survived.
/// 3. A wanted page is not a page. It has no tags and no path on disk, so no
///    filter can apply to it; it is returned wherever a link in the view
///    reaches it. The one thing it obeys is the walk, or `depth` would be a
///    promise broken at the edges.
///
/// Time is not in here at all. A page collects a time entry every time a timer
/// starts, so those edges would drown the links — see `/api/times?page=`.
#[utoipa::path(
    get,
    path = "/api/graph",
    tag = "graph",
    params(GraphQuery),
    responses(
        (status = 200, description = "Nodes and edges", body = GraphResponse),
        (status = 400, description = "`root` is not a valid slug", body = crate::error::ErrorResponse),
    ),
)]
pub async fn link_graph(
    State(state): State<AppState>,
    viewer: Viewer,
    Query(query): Query<GraphQuery>,
) -> AppResult<Json<GraphResponse>> {
    // Parsed, unlike `prefix` and `tag`: those are filters, where a value
    // nobody uses is an empty answer, and this one names a specific page.
    let root = query.root.as_deref().map(parse_slug).transpose()?;
    let depth = query.depth.unwrap_or(DEFAULT_DEPTH).min(MAX_DEPTH);
    let limit = query.limit.unwrap_or(DEFAULT_NODES).min(MAX_NODES);

    let graph = state
        .index
        .graph(
            GraphOptions {
                tag: query.tag,
                prefix: query.prefix,
                root: root.as_ref().map(Slug::to_string),
                depth,
                wanted: query.wanted.unwrap_or(true),
                limit,
            },
            &viewer.audience(),
        )
        .await?;

    Ok(Json(GraphResponse {
        nodes: graph
            .nodes
            .into_iter()
            .map(|node| GraphNodeView {
                slug: node.slug,
                title: node.title,
                exists: node.exists,
                inbound: node.inbound,
                outbound: node.outbound,
                tags: node.tags,
                distance: node.distance,
            })
            .collect(),
        edges: graph
            .edges
            .into_iter()
            .map(|edge| GraphEdgeView {
                source: edge.source,
                target: edge.target,
                kinds: edge
                    .kinds
                    .into_iter()
                    .map(|kind| kind.as_str().to_owned())
                    .collect(),
            })
            .collect(),
        matched: graph.matched,
        truncated: graph.truncated,
        depth: root.as_ref().map(|_| depth),
        root,
        limit,
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
pub async fn tags(State(state): State<AppState>, viewer: Viewer) -> AppResult<Json<TagsResponse>> {
    let tags = state.index.tags(&viewer.audience()).await?;

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
pub async fn stats(
    State(state): State<AppState>,
    viewer: Viewer,
) -> AppResult<Json<StatsResponse>> {
    let stats = state.index.stats(&viewer.audience()).await?;

    // Route hit counts are telemetry about the server rather than anything in
    // the wiki, and an anonymous reader of a few public pages has no business
    // with how often an agent has been calling `/api/reindex`. Empty rather than
    // absent, so the field's shape does not depend on who is asking.
    //
    // Both halves have to be withheld. `merge_usage` adds the in-memory tally to
    // the persisted counts, and skipping only the persisted half would still
    // hand out every route called since the last flush — which on a server that
    // has been up for under a minute is all of them.
    let usage = match viewer.is_permitted() {
        true => merge_usage(state.index.usage().await?, &state),
        false => Vec::new(),
    };

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
        api_usage: usage,
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
