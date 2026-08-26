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

use std::collections::{BTreeMap, BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use rusqlite::{Connection, OptionalExtension, ToSql, params};

use crate::index::audience::{self, Audience, VISIBLE};
use crate::index::schema::{KEY_LAST_SYNC, TOP_N};
use crate::index::{Index, IndexError, bindings, from_nanos};
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

/// How a whole-graph query is narrowed. Every field intersects.
#[derive(Debug, Clone)]
pub struct GraphOptions {
    /// Only pages carrying this tag.
    pub tag: Option<String>,
    /// Only pages at or under this slug path, hierarchically.
    pub prefix: Option<String>,
    /// Walk outward from this slug instead of taking the whole wiki.
    pub root: Option<String>,
    /// How many hops out from `root` the walk covers. Ignored without one.
    pub depth: usize,
    /// Whether pages that are linked to but not written are nodes.
    pub wanted: bool,
    /// How many *pages* the view may carry. The best-connected survive.
    pub limit: usize,
}

impl Default for GraphOptions {
    fn default() -> Self {
        Self {
            tag: None,
            prefix: None,
            root: None,
            depth: 2,
            wanted: true,
            limit: 500,
        }
    }
}

/// A page in the graph — possibly one nobody has written.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphNode {
    pub slug: String,
    /// The page's title, or the slug itself when there is no page.
    pub title: String,
    pub exists: bool,
    /// Distinct pages that name this one, counted across the **whole wiki**
    /// rather than the view. A hub therefore still looks like one inside a
    /// filter, and the gap between this number and the lines actually drawn at
    /// the node is itself the useful signal: it says the branch reaches outside.
    ///
    /// A `contents:` entry counts, because it is drawn. Leaving it out would
    /// make a book with sixty chapters the size of a leaf.
    pub inbound: usize,
    /// Distinct pages this one names, likewise wiki-wide.
    pub outbound: usize,
    pub tags: Vec<String>,
    /// Hops from `root`, when the query had one.
    pub distance: Option<usize>,
}

/// One line to draw: everything joining `source` to `target`, collapsed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphEdge {
    pub source: String,
    pub target: String,
    /// The kinds that reached it. Two, when a page is linked both as `[[a]]`
    /// and as `[a](a.md)` — which is two rows in `links` and one line to draw.
    pub kinds: Vec<LinkKind>,
    /// Whether the source's `contents:` list names the target.
    ///
    /// Kept apart from `kinds` rather than added to it as a sixth spelling of
    /// [`LinkKind`], because a part is **not** a kind of link and that is the
    /// decision `page_parts` exists to record. A page can be both linked to and
    /// assembled by the same parent, in which case one line carries both, and an
    /// edge that is only a part has no kinds at all.
    pub part: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Graph {
    pub nodes: Vec<GraphNode>,
    pub edges: Vec<GraphEdge>,
    /// Pages that matched the filters, before `limit` was applied.
    pub matched: usize,
    /// Whether `limit` dropped any of them.
    pub truncated: bool,
}

/// Only wiki and internal links are part of the page graph.
const IS_PAGE_LINK: &str = "links.kind != 'external'";

/// Everything joining one ordered pair of pages, before it becomes one line.
///
/// Two tables reach this: `links`, which contributes kinds, and `page_parts`,
/// which contributes a flag. They are collapsed together because the wiki has
/// one line to draw between two pages however many ways they are joined, and
/// kept as separate fields because a part is not a kind of link.
#[derive(Debug, Clone, Default)]
struct Joined {
    kinds: Vec<LinkKind>,
    part: bool,
}

/// A link both of whose ends this audience is allowed to know about.
///
/// Two conditions, and the second is the one that is easy to get wrong:
///
/// 1. **The source is a page they can read.** A backlink carries the linking
///    page's slug and title, so a link out of a private page is that page.
/// 2. **The target is not a page they cannot read.** Note the shape: it is not
///    "the target is visible", because a target that is not a page at all is a
///    [wanted page](crate::index::graph) and those are the whole point of the
///    graph. It is specifically that no *invisible* page sits there.
///
/// Getting the second one backwards is worse than leaving it out. If a link to a
/// private page merely lost its title, the page would appear in the graph as a
/// **wanted** page — drawn, named by its slug, and advertised as something worth
/// writing. A private page's slug is usually its title.
///
/// External links are unaffected: their target is a URL, which matches no slug,
/// so the subquery finds nothing and the link survives.
/// It takes no audience because it needs none: the predicate binds `:viewer` and
/// `:everything` by name, so the SQL is the same whoever is asking and only the
/// bindings differ.
fn visible_link() -> String {
    format!(
        "{IS_PAGE_LINK}
         and exists (
             select 1 from pages
             where pages.slug = links.src_slug and {VISIBLE}
         )
         and not exists (
             select 1 from pages as target
             where target.slug = links.target and not ({target})
         )",
        target = audience::visible_as("target"),
    )
}

/// A row in the spine this audience can see.
///
/// Both of [`visible_link`]'s conditions, for both of its reasons. The first is
/// obvious: a chapter list is written on a page, and naming the page is naming
/// it. The second is the one that is easy to leave out, and it became necessary
/// the moment these rows started being drawn: a `contents:` entry pointing at a
/// page the caller cannot read would otherwise appear as a **wanted** page,
/// named by its slug and advertised as a gap somebody should fill.
///
/// It was deliberately absent while the only question asked of these rows was
/// whether a page has a parent, which is answered entirely from the parent's
/// side. Adding it costs nothing there: the target of that question is a page
/// the caller can already see, so the condition is trivially true.
fn visible_part() -> String {
    format!(
        "exists (
             select 1 from pages as parent
             where parent.slug = page_parts.src_slug and {parent}
         )
         and not exists (
             select 1 from pages as chapter
             where chapter.slug = page_parts.target and not ({chapter})
         )",
        parent = audience::visible_as("parent"),
        chapter = audience::visible_as("chapter"),
    )
}

/// Whether a page is named by something: a link, or a contents list.
///
/// A chapter is referenced by the page that assembles it, so a manuscript's
/// chapters are not orphans. Without this the wiki's own meta-stats would fill
/// up with them the moment anybody wrote a book, and the orphan count is one of
/// the two numbers `/api/stats` exists for.
///
/// This is the union the plan describes, and it lives in one function because
/// the count and the list both ask it and must not drift.
fn referenced() -> String {
    format!(
        "exists (
             select 1 from links
             where links.target = pages.slug and {link}
         )
         or exists (
             select 1 from page_parts
             where page_parts.target = pages.slug and {part}
         )",
        link = visible_link(),
        part = visible_part(),
    )
}

impl Index {
    /// Every link into and out of a page.
    ///
    /// The page itself need not exist: asking about a wanted page returns the
    /// links pointing at it, which is exactly what you want to see before
    /// deciding to write it.
    pub async fn links_for(
        &self,
        slug: &Slug,
        audience: &Audience,
    ) -> Result<PageLinks, IndexError> {
        let slug = slug.to_string();
        let audience = audience.clone();

        self.with_connection(move |connection| {
            let visible = audience.params();
            let visible_target = audience::visible_as("target");

            // The join carries the audience predicate as well as the slug match,
            // so a target this caller cannot read contributes no title — and the
            // `not exists` below then removes the row entirely rather than
            // leaving it looking like a page nobody has written yet.
            let mut outbound_query = connection.prepare(&format!(
                "select links.target, links.display, links.kind, pages.title
                 from links
                 left join pages
                   on pages.slug = links.target and {IS_PAGE_LINK} and {VISIBLE}
                 where links.src_slug = :slug
                   and not exists (
                       select 1 from pages as target
                       where target.slug = links.target and not ({visible_target})
                   )
                 order by links.kind, links.target"
            ))?;

            let outbound = outbound_query
                .query_map(bindings(&[(":slug", &slug)], &visible).as_slice(), |row| {
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

            // A backlink names the page that wrote it and shows its title, so an
            // unfiltered inbound list is a directory of every private page that
            // happens to link here.
            let mut inbound_query = connection.prepare(&format!(
                "select links.src_slug, pages.title, links.display, links.kind
                 from links
                 join pages on pages.slug = links.src_slug
                 where links.target = :slug and {IS_PAGE_LINK} and {VISIBLE}
                 order by links.src_slug"
            ))?;

            let inbound = inbound_query
                .query_map(bindings(&[(":slug", &slug)], &visible).as_slice(), |row| {
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

    /// The link graph as nodes and edges, narrowed by [`GraphOptions`].
    ///
    /// Three rules decide what comes back, and the third is the one that is not
    /// obvious:
    ///
    /// 1. The filters select **pages**.
    /// 2. An edge is drawn when both of its ends survived.
    /// 3. **A wanted page is not a page.** It has no row anywhere — no tags, no
    ///    path on disk, nothing a filter could ask about — so the filters
    ///    cannot apply to it. It is drawn wherever a link in the view reaches
    ///    it, and that is the whole reason it is worth drawing: it is a branch
    ///    someone gestured at, and a view that hid it would be claiming the
    ///    links in it all land somewhere.
    ///
    /// The one thing a wanted page is still subject to is the walk: with a
    /// `root`, everything drawn is within `depth` hops of it, or the promise the
    /// parameter makes would be false at the edges.
    pub async fn graph(
        &self,
        options: GraphOptions,
        audience: &Audience,
    ) -> Result<Graph, IndexError> {
        let audience = audience.clone();

        self.with_connection(move |connection| {
            let GraphOptions {
                tag,
                prefix,
                root,
                depth,
                wanted,
                limit,
            } = options;
            let visible = audience.params();
            let visible_link = visible_link();
            let visible_part = visible_part();

            // Every page link **this audience may see**, collapsed to one entry
            // per ordered pair. Loaded whole rather than filtered further in SQL
            // because the degrees below count the whole wiki, so a filtered
            // query would only have to be run a second time.
            //
            // "The whole wiki" now means the whole wiki as this caller sees it.
            // That is the only coherent answer — a degree that counted links
            // from pages they cannot read would be a number about pages they
            // cannot read — and it means two accounts can legitimately see
            // different degrees for the same page.
            let mut collapsed: BTreeMap<(String, String), Joined> = BTreeMap::new();
            {
                let mut query = connection.prepare(&format!(
                    "select src_slug, target, kind from links
                     where {visible_link}
                     order by src_slug, target, kind"
                ))?;
                let rows = query.query_map(visible.as_slice(), |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                })?;
                for row in rows {
                    let (source, target, kind) = row?;
                    let Some(kind) = LinkKind::parse(&kind) else {
                        continue;
                    };
                    collapsed
                        .entry((source, target))
                        .or_default()
                        .kinds
                        .push(kind);
                }
            }

            // The spine, on the same footing. This is the half of the L1 plan
            // that waited for a panel to exist: the orphan count needed the
            // union immediately, because a chapter reported as unreferenced is a
            // number being wrong, and drawing it needed somebody to have decided
            // what a part edge looks like.
            //
            // A page listed twice under one parent is one line, because position
            // is what the manifest is for and two lines between the same pair
            // would say nothing the first does not.
            {
                let mut query = connection.prepare(&format!(
                    "select src_slug, target from page_parts
                     where {visible_part}
                     order by src_slug, ordinal"
                ))?;
                let rows = query.query_map(visible.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (source, target) = row?;
                    // `page_parts.target` is the entry as it was written, so a
                    // mistyped one is in there: `../etc/passwd` is `invalid` in
                    // the manifest and must not become a node here, because a
                    // wanted node is an invitation to write the page. A valid
                    // slug nobody has written is a different thing entirely, and
                    // it is exactly the gap the graph should show.
                    if Slug::parse(&target).is_err() {
                        continue;
                    }
                    collapsed.entry((source, target)).or_default().part = true;
                }
            }

            // (inbound, outbound), wiki-wide, counted over collapsed pairs so a
            // page linked twice over is one referrer rather than two, and a page
            // both linked to and assembled by one parent is one as well.
            let mut degrees: HashMap<String, (usize, usize)> = HashMap::new();
            for (source, target) in collapsed.keys() {
                degrees.entry(source.clone()).or_default().1 += 1;
                degrees.entry(target.clone()).or_default().0 += 1;
            }

            // Doubles as the set of slugs that name a page that exists, which
            // is what separates "excluded by a filter" from "never written".
            // Only the visible ones, which keeps its second job honest too: a
            // page that is here is one that exists *as far as this caller is
            // concerned*, and one that is not is either unwritten or none of
            // their business. The two are indistinguishable from outside, and
            // that is the point.
            let mut titles: HashMap<String, String> = HashMap::new();
            {
                let mut query = connection
                    .prepare(&format!("select slug, title from pages where {VISIBLE}"))?;
                let rows = query.query_map(visible.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (slug, title) = row?;
                    titles.insert(slug, title);
                }
            }

            let distances = match &root {
                Some(root) => Some(walk_from(
                    connection,
                    root,
                    depth,
                    &visible_link,
                    &visible_part,
                    &visible,
                )?),
                None => None,
            };

            // The same filter expression the listing uses, and for the same
            // reasons — `substr` rather than `like` because slugs are
            // case-sensitive and a prefix has to stop at the separator.
            let mut matched: Vec<String> = {
                let mut query = connection.prepare(&format!(
                    "select slug from pages
                     where (:tag is null or exists (
                              select 1 from page_tags
                              where page_tags.slug = pages.slug and page_tags.tag = :tag
                          ))
                       and (:prefix is null
                            or slug = :prefix
                            or substr(slug, 1, length(:prefix) + 1) = :prefix || '/')
                       and {VISIBLE}
                     order by slug"
                ))?;
                query
                    .query_map(
                        bindings(&[(":tag", &tag), (":prefix", &prefix)], &visible).as_slice(),
                        |row| row.get::<_, String>(0),
                    )?
                    .collect::<Result<Vec<_>, _>>()?
            };

            if let Some(distances) = &distances {
                matched.retain(|slug| distances.contains_key(slug));
            }

            let matched_count = matched.len();
            let truncated = matched_count > limit;
            if truncated {
                // Drop leaves, not hubs. A graph cut down to its least
                // connected pages is a scatter of dots that says nothing.
                let degree = |slug: &String| {
                    degrees
                        .get(slug)
                        .map_or(0, |(inbound, outbound)| inbound + outbound)
                };
                matched.sort_by(|a, b| degree(b).cmp(&degree(a)).then_with(|| a.cmp(b)));
                matched.truncate(limit);
                matched.sort();
            }

            let pages: BTreeSet<String> = matched.into_iter().collect();

            let mut edges = Vec::new();
            let mut unwritten: BTreeSet<String> = BTreeSet::new();
            for ((source, target), joined) in &collapsed {
                if !pages.contains(source) {
                    continue;
                }

                if !pages.contains(target) {
                    // A page that exists but the filters excluded: its edge is
                    // out of the view, not a want.
                    if titles.contains_key(target) || !wanted {
                        continue;
                    }
                    if distances
                        .as_ref()
                        .is_some_and(|reached| !reached.contains_key(target))
                    {
                        continue;
                    }
                    unwritten.insert(target.clone());
                }

                edges.push(GraphEdge {
                    source: source.clone(),
                    target: target.clone(),
                    kinds: joined.kinds.clone(),
                    part: joined.part,
                });
            }

            let mut tags_of: HashMap<String, Vec<String>> = HashMap::new();
            {
                let mut query = connection.prepare(&format!(
                    "select page_tags.slug, page_tags.tag
                     from page_tags
                     join pages on pages.slug = page_tags.slug
                     where {VISIBLE}
                     order by page_tags.slug, page_tags.tag"
                ))?;
                let rows = query.query_map(visible.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (slug, tag) = row?;
                    tags_of.entry(slug).or_default().push(tag);
                }
            }

            let node = |slug: String, exists: bool| {
                let (inbound, outbound) = degrees.get(&slug).copied().unwrap_or_default();
                GraphNode {
                    title: match exists {
                        true => titles.get(&slug).cloned().unwrap_or_else(|| slug.clone()),
                        false => slug.clone(),
                    },
                    exists,
                    inbound,
                    outbound,
                    tags: tags_of.get(&slug).cloned().unwrap_or_default(),
                    distance: distances
                        .as_ref()
                        .and_then(|reached| reached.get(&slug).copied()),
                    slug,
                }
            };

            let mut nodes: Vec<GraphNode> = pages
                .into_iter()
                .map(|slug| node(slug, true))
                .chain(unwritten.into_iter().map(|slug| node(slug, false)))
                .collect();
            nodes.sort_by(|a, b| a.slug.cmp(&b.slug));

            Ok(Graph {
                nodes,
                edges,
                matched: matched_count,
                truncated,
            })
        })
        .await
    }

    /// Every tag in the wiki, most-used first.
    ///
    /// Counted over the pages this audience can read, so a tag used only by
    /// pages they cannot see does not appear at all. A count is a small leak and
    /// a tag nobody else uses is a large one — `tags: [acquisition]` on three
    /// private pages would otherwise show up as a tag with three pages behind it.
    pub async fn tags(&self, audience: &Audience) -> Result<Vec<TagCount>, IndexError> {
        let audience = audience.clone();

        self.with_connection(move |connection| {
            let visible = audience.params();
            let mut query = connection.prepare(&format!(
                "select page_tags.tag, count(*) as pages
                 from page_tags
                 join pages on pages.slug = page_tags.slug
                 where {VISIBLE}
                 group by page_tags.tag
                 order by pages desc, page_tags.tag asc"
            ))?;

            let tags = query
                .query_map(visible.as_slice(), |row| {
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
    ///
    /// **Every figure here is computed over the subgraph this audience can see.**
    /// That is not a caveat, it is the definition: each of these six numbers is a
    /// different way to describe pages, and a total that counted private ones
    /// would be a statement about them.
    ///
    /// One consequence is worth stating because it looks like a bug. A page that
    /// is linked only from a page you cannot read is an **orphan to you** and not
    /// to its owner. Both answers are correct; they are answers to different
    /// questions, and the alternative — reporting a page as linked without being
    /// able to say from where — is how you learn that something you cannot see
    /// points at it.
    pub async fn stats(&self, audience: &Audience) -> Result<Stats, IndexError> {
        let audience = audience.clone();

        self.with_connection(move |connection| {
            let visible = audience.params();
            let visible_link = visible_link();
            // The link half of the audience filter, for the subqueries that ask
            // about a link's target from inside a query over `pages`.
            let is_visible_page = VISIBLE;

            let pages: i64 = connection.query_row(
                &format!("select count(*) from pages where {is_visible_page}"),
                visible.as_slice(),
                |row| row.get(0),
            )?;

            // External links are counted only from pages the caller can read.
            // The URL itself gives nothing away; which page cites it does.
            let (internal, external): (i64, i64) = connection.query_row(
                &format!(
                    "select
                       coalesce(sum(case when links.kind != 'external' then 1 else 0 end), 0),
                       coalesce(sum(case when links.kind = 'external' then 1 else 0 end), 0)
                     from links
                     where exists (
                         select 1 from pages
                         where pages.slug = links.src_slug and {is_visible_page}
                     )
                     and not exists (
                         select 1 from pages as target
                         where target.slug = links.target
                           and links.kind != 'external'
                           and not ({visible_target})
                     )",
                    visible_target = audience::visible_as("target"),
                ),
                visible.as_slice(),
                |row| Ok((row.get(0)?, row.get(1)?)),
            )?;

            let resolved: i64 = connection.query_row(
                &format!(
                    "select count(*) from links
                     where {visible_link}
                       and exists (
                           select 1 from pages
                           where pages.slug = links.target and {is_visible_page}
                       )"
                ),
                visible.as_slice(),
                |row| row.get(0),
            )?;

            // A link to a page this caller cannot read is already gone from
            // `visible_link`, so it is not counted here — which is the whole
            // point. Counting it would report a private page as one that wants
            // writing, and name it.
            let wanted_count: i64 = connection.query_row(
                &format!(
                    "select count(distinct links.target) from links
                     where {visible_link}
                       and not exists (select 1 from pages where pages.slug = links.target)"
                ),
                visible.as_slice(),
                |row| row.get(0),
            )?;

            let orphan_count: i64 = connection.query_row(
                &format!(
                    "select count(*) from pages
                     where {is_visible_page}
                       and not ({referenced})",
                    referenced = referenced()
                ),
                visible.as_slice(),
                |row| row.get(0),
            )?;

            let top_n = TOP_N as i64;

            let mut wanted_query = connection.prepare(&format!(
                "select links.target, count(distinct links.src_slug) as referrers
                 from links
                 where {visible_link}
                   and not exists (select 1 from pages where pages.slug = links.target)
                 group by links.target
                 order by referrers desc, links.target asc
                 limit :top_n"
            ))?;
            let wanted = wanted_query
                .query_map(
                    bindings(&[(":top_n", &top_n)], &visible).as_slice(),
                    |row| {
                        Ok(WantedPage {
                            slug: row.get(0)?,
                            referrers: row.get::<_, i64>(1)? as usize,
                        })
                    },
                )?
                .collect::<Result<Vec<_>, _>>()?;

            let mut orphan_query = connection.prepare(&format!(
                "select pages.slug, pages.title
                 from pages
                 where {is_visible_page}
                   and not ({referenced})
                 order by pages.slug
                 limit :top_n",
                referenced = referenced()
            ))?;
            let orphans = orphan_query
                .query_map(
                    bindings(&[(":top_n", &top_n)], &visible).as_slice(),
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?
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
                 where {visible_link} and {is_visible_page}
                 group by links.target, pages.title
                 order by referrers desc, links.target asc
                 limit :top_n"
            ))?;
            let most_linked = linked_query
                .query_map(
                    bindings(&[(":top_n", &top_n)], &visible).as_slice(),
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, i64>(2)?,
                        ))
                    },
                )?
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

            let mut tag_query = connection.prepare(&format!(
                "select page_tags.tag, count(*) as pages
                 from page_tags
                 join pages on pages.slug = page_tags.slug
                 where {is_visible_page}
                 group by page_tags.tag
                 order by pages desc, page_tags.tag asc"
            ))?;
            let tag_counts = tag_query
                .query_map(visible.as_slice(), |row| {
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

/// Every slug within `depth` hops of `root`, with the fewest hops that reaches
/// it.
///
/// Direction is ignored on purpose. A page's neighbourhood is what it points at
/// *and* what points at it — the same reason `/api/links/{slug}` returns both
/// directions from one call — so the walk crosses every edge either way.
///
/// The root need not name a page that exists: a wanted page has a
/// neighbourhood, and it is exactly the set of pages waiting on it.
/// The walk crosses only links the audience may see, which is what stops a
/// neighbourhood being routed *through* a page they cannot read: without it, two
/// pages joined only by a private one would look adjacent, and the private page's
/// existence would be legible from the shape of the graph.
///
/// **It crosses the spine too**, so a chapter's neighbourhood holds the book it
/// belongs to. Leaving `page_parts` out would make the one page a chapter is
/// certain to be connected to the one page a walk could not reach.
fn walk_from(
    connection: &Connection,
    root: &str,
    depth: usize,
    visible_link: &str,
    visible_part: &str,
    visible: &[(&'static str, &dyn ToSql); 2],
) -> Result<HashMap<String, usize>, IndexError> {
    // `union` rather than `union all` is what stops a cycle looping forever:
    // it drops rows already produced. A node can still be reached at two
    // different distances, which is why the outer query takes the smaller.
    //
    // `joined` is a plain, non-recursive term in the same `with recursive`
    // clause, which is what lets the walk cross links and contents entries
    // without the recursive half having to name two tables.
    let mut query = connection.prepare(&format!(
        "with recursive
         joined(src_slug, target) as (
             select src_slug, target from links where {visible_link}
             union
             select src_slug, target from page_parts where {visible_part}
         ),
         walk(slug, distance) as (
             select :root, 0
             union
             select case when joined.src_slug = walk.slug
                         then joined.target
                         else joined.src_slug
                    end,
                    walk.distance + 1
             from joined
             join walk on joined.src_slug = walk.slug or joined.target = walk.slug
             where walk.distance < :depth
         )
         select slug, min(distance) from walk group by slug"
    ))?;

    let depth = depth as i64;
    let rows = query.query_map(
        bindings(&[(":root", &root), (":depth", &depth)], visible).as_slice(),
        |row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?)),
    )?;

    let mut reached = HashMap::new();
    for row in rows {
        let (slug, distance) = row?;
        reached.insert(slug, distance.max(0) as usize);
    }
    Ok(reached)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::page::{Frontmatter, Page};
    /// Every test in this file runs against a wiki with no accounts, where
    /// there is nobody to keep a page from and visibility does not apply.
    /// What happens when it does is `tests/visibility.rs`, which is a whole
    /// file rather than a case here for exactly that reason.
    const EVERYONE: Audience = Audience::Everything;

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
                ..Frontmatter::default()
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
            .links_for(&Slug::parse("index").unwrap(), &EVERYONE)
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
            .links_for(&Slug::parse("notes/target").unwrap(), &EVERYONE)
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

        let before = index.stats(&EVERYONE).await.unwrap();
        assert_eq!(before.links.wanted, 1);
        assert_eq!(before.links.resolved, 0);
        assert_eq!(before.wanted[0].slug, "notes/later");

        // Only the new page is written. Nothing touches `a`.
        seed(&index, "notes/later", "Now it exists.\n").await;

        let after = index.stats(&EVERYONE).await.unwrap();
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
        assert_eq!(index.stats(&EVERYONE).await.unwrap().links.resolved, 1);

        index
            .remove(&Slug::parse("notes/target").unwrap())
            .await
            .unwrap();

        let stats = index.stats(&EVERYONE).await.unwrap();
        assert_eq!(stats.links.wanted, 1);
        assert_eq!(stats.wanted[0].slug, "notes/target");
        // `a` still has its outbound link; it just points at nothing now.
        let links = index
            .links_for(&Slug::parse("a").unwrap(), &EVERYONE)
            .await
            .unwrap();
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
            .links_for(&Slug::parse("target").unwrap(), &EVERYONE)
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

        let stats = index.stats(&EVERYONE).await.unwrap();

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

        let stats = index.stats(&EVERYONE).await.unwrap();

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

        let tags = index.tags(&EVERYONE).await.unwrap();

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
        assert_eq!(index.stats(&EVERYONE).await.unwrap().tags, 2);
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

        let stats = index.stats(&EVERYONE).await.unwrap();

        assert_eq!(stats.links.internal, 1);
        assert_eq!(stats.links.external, 2);
    }

    #[tokio::test]
    async fn an_empty_wiki_has_empty_stats() {
        let stats = index().await.stats(&EVERYONE).await.unwrap();

        assert_eq!(stats.pages, 0);
        assert_eq!(stats.links, LinkTotals::default());
        assert!(stats.orphans.is_empty());
        assert!(stats.wanted.is_empty());
        assert!(stats.tag_counts.is_empty());
    }

    /// A wiki with a hub, a leaf hanging off it, a want, and an island.
    async fn graph_index() -> Index {
        let index = index().await;
        seed(
            &index,
            "index",
            "See [[notes/rust]] and [[notes/rhizome]].\n",
        )
        .await;
        seed(
            &index,
            "notes/rust",
            "Async is [[notes/rust/async]], and [[notes/rust/streams]] is not written.\n",
        )
        .await;
        seed(&index, "notes/rust/async", "Back to [[notes/rust]].\n").await;
        seed(&index, "notes/rhizome", "Theory.\n").await;
        seed(
            &index,
            "scratch/inbox",
            "Nothing links here and it links nowhere.\n",
        )
        .await;
        index
    }

    fn slugs(graph: &Graph) -> Vec<&str> {
        graph.nodes.iter().map(|node| node.slug.as_str()).collect()
    }

    fn pairs(graph: &Graph) -> Vec<(&str, &str)> {
        graph
            .edges
            .iter()
            .map(|edge| (edge.source.as_str(), edge.target.as_str()))
            .collect()
    }

    #[tokio::test]
    async fn the_whole_graph_carries_orphans_and_wants_alike() {
        let graph = graph_index()
            .await
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        assert_eq!(
            slugs(&graph),
            [
                "index",
                "notes/rhizome",
                "notes/rust",
                "notes/rust/async",
                "notes/rust/streams",
                "scratch/inbox",
            ],
            "an orphan is a node with no lines at it, not an absent node"
        );

        let streams = &graph.nodes[4];
        assert!(!streams.exists, "nobody has written it");
        assert_eq!(streams.inbound, 1);
        assert_eq!(streams.outbound, 0);

        assert_eq!(
            pairs(&graph),
            [
                ("index", "notes/rhizome"),
                ("index", "notes/rust"),
                ("notes/rust", "notes/rust/async"),
                ("notes/rust", "notes/rust/streams"),
                ("notes/rust/async", "notes/rust"),
            ],
            "a mutual pair is two directed edges, not one"
        );
    }

    #[tokio::test]
    async fn links_of_two_kinds_between_one_pair_are_one_edge() {
        let index = index().await;
        seed(&index, "b", "Target.\n").await;
        seed(&index, "a", "Both [[b]] and [b](b.md).\n").await;

        let graph = index
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        assert_eq!(graph.edges.len(), 1, "one line to draw");
        assert_eq!(graph.edges[0].kinds, [LinkKind::Internal, LinkKind::Wiki]);
        // ...and it is one referrer, not two, so the degree matches the picture.
        let b = graph.nodes.iter().find(|node| node.slug == "b").unwrap();
        assert_eq!(b.inbound, 1);
    }

    #[tokio::test]
    async fn a_filter_selects_pages_and_edges_need_both_ends() {
        let graph = graph_index()
            .await
            .graph(
                GraphOptions {
                    prefix: Some("notes/rust".to_owned()),
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert_eq!(
            slugs(&graph),
            ["notes/rust", "notes/rust/async", "notes/rust/streams"]
        );
        // `index` links to `notes/rust`, and that edge is out of the view
        // because only one of its ends is in it.
        assert_eq!(
            pairs(&graph),
            [
                ("notes/rust", "notes/rust/async"),
                ("notes/rust", "notes/rust/streams"),
                ("notes/rust/async", "notes/rust"),
            ]
        );

        // The degree still counts the whole wiki, which is what says the branch
        // is not closed: one line arrives at `notes/rust` in the picture, and
        // two pages link to it.
        let rust = &graph.nodes[0];
        assert_eq!(rust.inbound, 2);
    }

    /// The rule that is not obvious: a wanted page has nothing to filter on.
    #[tokio::test]
    async fn a_wanted_page_is_not_filtered_because_it_is_not_a_page() {
        let index = index().await;
        index
            .upsert(&page(
                "notes/rust",
                &["rust"],
                "Wants [[code/rust/streams]].\n",
            ))
            .await
            .unwrap();
        index
            .upsert(&page("notes/rhizome", &[], "Untagged.\n"))
            .await
            .unwrap();

        let graph = index
            .graph(
                GraphOptions {
                    tag: Some("rust".to_owned()),
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        // The want survives a tag filter it could never satisfy, and a prefix
        // it sits outside of. Hiding it would claim every link in the view
        // lands somewhere.
        assert_eq!(slugs(&graph), ["code/rust/streams", "notes/rust"]);
    }

    #[tokio::test]
    async fn wants_can_be_left_out() {
        let graph = graph_index()
            .await
            .graph(
                GraphOptions {
                    wanted: false,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert!(!slugs(&graph).contains(&"notes/rust/streams"));
        assert!(
            !pairs(&graph).contains(&("notes/rust", "notes/rust/streams")),
            "an edge to a node nobody drew is a line into empty space"
        );
    }

    #[tokio::test]
    async fn a_walk_goes_both_ways_and_stops_at_the_depth() {
        let index = graph_index().await;

        let one = index
            .graph(
                GraphOptions {
                    root: Some("notes/rust/async".to_owned()),
                    depth: 1,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();
        assert_eq!(slugs(&one), ["notes/rust", "notes/rust/async"]);

        // Two hops reaches `index` — which `notes/rust` does not link to, and
        // which links to it. A walk that only followed arrows forward would
        // never find it, and a page's neighbourhood is both directions.
        let two = index
            .graph(
                GraphOptions {
                    root: Some("notes/rust/async".to_owned()),
                    depth: 2,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();
        assert_eq!(
            slugs(&two),
            [
                "index",
                "notes/rust",
                "notes/rust/async",
                "notes/rust/streams"
            ]
        );

        let distances: Vec<Option<usize>> = two.nodes.iter().map(|node| node.distance).collect();
        assert_eq!(distances, [Some(2), Some(1), Some(0), Some(2)]);
        assert!(
            !slugs(&two).contains(&"notes/rhizome"),
            "three hops away, and the depth was two"
        );
    }

    /// A wanted page is exempt from the filters but not from the walk, or
    /// `depth` would be a promise the edges break.
    #[tokio::test]
    async fn a_walk_bounds_wants_too() {
        let index = graph_index().await;

        let one = index
            .graph(
                GraphOptions {
                    root: Some("index".to_owned()),
                    depth: 1,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert_eq!(slugs(&one), ["index", "notes/rhizome", "notes/rust"]);
        assert!(
            !slugs(&one).contains(&"notes/rust/streams"),
            "two hops out, reached through a page that is one hop out"
        );
    }

    #[tokio::test]
    async fn a_wanted_page_has_a_neighbourhood_of_its_own() {
        let graph = graph_index()
            .await
            .graph(
                GraphOptions {
                    root: Some("notes/rust/streams".to_owned()),
                    depth: 1,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert_eq!(slugs(&graph), ["notes/rust", "notes/rust/streams"]);
        assert!(!graph.nodes[1].exists);
    }

    #[tokio::test]
    async fn truncation_keeps_the_hubs() {
        let graph = graph_index()
            .await
            .graph(
                GraphOptions {
                    limit: 2,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert!(graph.truncated);
        assert_eq!(graph.matched, 5, "matched counts before the cap");
        // `notes/rust` (four) and `index` (two) are the best-connected; the
        // island goes first.
        assert_eq!(slugs(&graph), ["index", "notes/rust", "notes/rust/streams"]);
        assert!(!slugs(&graph).contains(&"scratch/inbox"));
    }

    // ------------------------------------------------------------- the spine

    fn contents(slug: &str, parts: &[&str], body: &str) -> Page {
        let mut page = page(slug, &[], body);
        page.frontmatter.contents = Some(parts.iter().map(|part| (*part).to_string()).collect());
        page
    }

    /// A book with two chapters, one of them unwritten, and an appendix listed
    /// under both parts. Nothing in it links to anything.
    async fn book() -> Index {
        let index = index().await;
        index
            .upsert(&contents("book", &["book/one", "book/two"], "# The book\n"))
            .await
            .expect("upsert");
        index
            .upsert(&contents(
                "book/one",
                &["book/one/opening", "book/appendix"],
                "## Part one\n",
            ))
            .await
            .expect("upsert");
        index
            .upsert(&contents("book/two", &["book/appendix"], "## Part two\n"))
            .await
            .expect("upsert");
        index
            .upsert(&page("book/one/opening", &[], "The ferry was late.\n"))
            .await
            .expect("upsert");
        index
            .upsert(&page("book/appendix", &[], "Sources.\n"))
            .await
            .expect("upsert");
        index
    }

    /// The half of the L1 plan that waited for somebody to decide what a part
    /// edge looks like.
    #[tokio::test]
    async fn a_contents_entry_is_an_edge_and_says_it_is_a_part() {
        let graph = book()
            .await
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        assert_eq!(
            pairs(&graph),
            [
                ("book", "book/one"),
                ("book", "book/two"),
                ("book/one", "book/appendix"),
                ("book/one", "book/one/opening"),
                ("book/two", "book/appendix"),
            ]
        );
        assert!(
            graph
                .edges
                .iter()
                .all(|edge| edge.part && edge.kinds.is_empty()),
            "nothing in this book links to anything, so every line is a part"
        );

        // An appendix under two parts is two lines, which is what makes a
        // diamond visible rather than a mystery about why one part is short.
        let appendix = graph
            .nodes
            .iter()
            .find(|node| node.slug == "book/appendix")
            .unwrap();
        assert_eq!(appendix.inbound, 2);
    }

    /// A gap in a manuscript is a branch somebody gestured at, which is exactly
    /// what a wanted node already means.
    #[tokio::test]
    async fn a_chapter_nobody_has_written_is_a_wanted_node() {
        let index = book().await;
        index
            .upsert(&contents(
                "book/two",
                &["book/two/the-ferry"],
                "## Part two\n",
            ))
            .await
            .unwrap();

        let graph = index
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        let ferry = graph
            .nodes
            .iter()
            .find(|node| node.slug == "book/two/the-ferry")
            .expect("drawn");
        assert!(!ferry.exists);
        assert!(pairs(&graph).contains(&("book/two", "book/two/the-ferry")));
    }

    /// `page_parts.target` is the entry as it was written, so a mistyped one is
    /// in the table. Drawing it would advertise a path as a page worth writing.
    #[tokio::test]
    async fn a_contents_entry_that_is_not_a_slug_is_not_a_node() {
        let index = index().await;
        index
            .upsert(&contents(
                "book",
                &["../etc/passwd", "book/one"],
                "# The book\n",
            ))
            .await
            .unwrap();

        let graph = index
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        assert_eq!(slugs(&graph), ["book", "book/one"]);
        assert_eq!(pairs(&graph), [("book", "book/one")]);
    }

    /// One line, carrying both, because the wiki has one relationship to draw
    /// between two pages however many ways they are joined.
    #[tokio::test]
    async fn a_chapter_that_is_also_linked_is_one_edge_carrying_both() {
        let index = index().await;
        index
            .upsert(&contents(
                "book",
                &["book/one"],
                "# The book\n\nSee [[book/one]].\n",
            ))
            .await
            .unwrap();
        index
            .upsert(&page("book/one", &[], "Part one.\n"))
            .await
            .unwrap();

        let graph = index
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        assert_eq!(graph.edges.len(), 1);
        assert!(graph.edges[0].part);
        assert_eq!(graph.edges[0].kinds, [LinkKind::Wiki]);
        // ...and one referrer, so the degree matches the picture.
        let one = graph.nodes.iter().find(|n| n.slug == "book/one").unwrap();
        assert_eq!(one.inbound, 1);
    }

    /// The one page a chapter is certain to be connected to must not be the one
    /// page a walk cannot reach.
    #[tokio::test]
    async fn a_walk_climbs_the_spine() {
        let graph = book()
            .await
            .graph(
                GraphOptions {
                    root: Some("book/one/opening".to_owned()),
                    depth: 2,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert_eq!(
            slugs(&graph),
            ["book", "book/appendix", "book/one", "book/one/opening"],
            "one hop to its part, two to the book and to what else the part holds"
        );
        let distances: Vec<Option<usize>> = graph.nodes.iter().map(|node| node.distance).collect();
        assert_eq!(distances, [Some(2), Some(2), Some(1), Some(0)]);
    }

    #[tokio::test]
    async fn an_empty_wiki_draws_nothing() {
        let graph = index()
            .await
            .graph(GraphOptions::default(), &EVERYONE)
            .await
            .unwrap();

        assert!(graph.nodes.is_empty());
        assert!(graph.edges.is_empty());
        assert!(!graph.truncated);
    }

    #[tokio::test]
    async fn a_root_nothing_knows_about_is_a_graph_of_one() {
        let graph = graph_index()
            .await
            .graph(
                GraphOptions {
                    root: Some("nowhere".to_owned()),
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert!(graph.nodes.is_empty(), "no page, and nothing points at it");
        assert!(graph.edges.is_empty());
    }

    /// A page linking to itself is a cycle of length one, and the walk has to
    /// terminate on it rather than recursing to the depth limit forever.
    #[tokio::test]
    async fn a_self_link_terminates() {
        let index = index().await;
        seed(&index, "a", "See [[a]].\n").await;

        let graph = index
            .graph(
                GraphOptions {
                    root: Some("a".to_owned()),
                    depth: 4,
                    ..GraphOptions::default()
                },
                &EVERYONE,
            )
            .await
            .unwrap();

        assert_eq!(slugs(&graph), ["a"]);
        assert_eq!(graph.nodes[0].distance, Some(0));
        assert_eq!(pairs(&graph), [("a", "a")]);
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

        assert_eq!(index.count(&EVERYONE).await.unwrap(), 0);
        assert_eq!(index.usage().await.unwrap()[0].count, 7);
    }
}
