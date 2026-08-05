//! Counting API calls, for the dashboard's usage stats.
//!
//! Counts are tallied in memory and flushed to SQLite in batches. One database
//! write per request would be absurd for a number nobody reads in real time.
//!
//! The key is axum's [`MatchedPath`] — the route *template* (`/api/pages/{slug}`)
//! rather than the URL that arrived. Keying on the raw URI would grow a row per
//! page ever fetched, which is unbounded and useless.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use axum::extract::{MatchedPath, Request, State};
use axum::middleware::Next;
use axum::response::Response;

use crate::api::AppState;

/// Route and method, e.g. `("/api/pages/{slug}", "GET")`.
pub type RouteKey = (String, String);

/// An in-memory tally of API calls since the last flush.
#[derive(Clone, Default)]
pub struct UsageTally {
    counts: Arc<Mutex<HashMap<RouteKey, u64>>>,
}

impl UsageTally {
    pub fn new() -> Self {
        Self::default()
    }

    fn record(&self, route: String, method: String) {
        let mut counts = self.lock();
        *counts.entry((route, method)).or_insert(0) += 1;
    }

    /// Take everything tallied so far, leaving the tally empty.
    ///
    /// Used by the flusher. Anything returned here is the caller's
    /// responsibility to persist — dropping it loses those counts.
    pub fn drain(&self) -> Vec<(RouteKey, u64)> {
        self.lock().drain().collect()
    }

    /// Read the tally without clearing it.
    ///
    /// `/api/stats` adds this to the persisted counts, so the number it reports
    /// is current without the read having to write anything.
    pub fn snapshot(&self) -> Vec<(RouteKey, u64)> {
        self.lock()
            .iter()
            .map(|(key, count)| (key.clone(), *count))
            .collect()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<RouteKey, u64>> {
        // A counter is not worth poisoning the process over.
        self.counts
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }
}

/// Middleware that counts every request against its matched route.
pub async fn count(
    State(state): State<AppState>,
    matched: Option<MatchedPath>,
    request: Request,
    next: Next,
) -> Response {
    // Both have to be read before the request is consumed by `next`.
    let route = matched.map(|matched| normalize(matched.as_str()));
    let method = request.method().to_string();

    let response = next.run(request).await;

    if let Some(route) = route {
        state.usage.record(route, method);
    }
    response
}

/// Report the route the way the OpenAPI document spells it.
///
/// axum matches `/api/pages/{*slug}`; the published spec says
/// `/api/pages/{slug}`. Usage stats that disagree with the docs about what an
/// endpoint is called are a small, needless puzzle.
fn normalize(route: &str) -> String {
    route.replace("{*", "{")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tallies_per_route_and_method() {
        let tally = UsageTally::new();

        tally.record("/api/pages".into(), "GET".into());
        tally.record("/api/pages".into(), "GET".into());
        tally.record("/api/pages".into(), "POST".into());

        let mut snapshot = tally.snapshot();
        snapshot.sort();

        assert_eq!(
            snapshot,
            [
                (("/api/pages".to_owned(), "GET".to_owned()), 2),
                (("/api/pages".to_owned(), "POST".to_owned()), 1),
            ]
        );
    }

    #[test]
    fn a_snapshot_leaves_the_tally_intact_and_a_drain_empties_it() {
        let tally = UsageTally::new();
        tally.record("/api/pages".into(), "GET".into());

        assert_eq!(tally.snapshot().len(), 1);
        assert_eq!(tally.snapshot().len(), 1, "snapshot consumed the tally");

        assert_eq!(tally.drain().len(), 1);
        assert!(tally.snapshot().is_empty(), "drain left counts behind");
    }

    #[test]
    fn routes_are_reported_the_way_the_spec_spells_them() {
        assert_eq!(normalize("/api/pages/{*slug}"), "/api/pages/{slug}");
        assert_eq!(normalize("/api/pages"), "/api/pages");
    }
}
