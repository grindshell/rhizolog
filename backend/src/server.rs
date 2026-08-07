//! Starting and stopping the server.
//!
//! The boot sequence lives here rather than in `main` because more than one
//! thing needs to drive it, and the callers disagree only about the very end.
//! A console server stops on Ctrl-C, a desktop shell stops when its window
//! closes, and a test stops when it has finished asserting; everything up to
//! that point is identical, and duplicating it is how the two would drift.
//!
//! [`start`] returns once the server is **ready** — bound, reconciled, and
//! watching — rather than once it has begun starting. That matters because the
//! bound address is not necessarily the one that was asked for, and a caller
//! that has to tell somebody else where to connect needs something true to
//! tell them. See `knowledge-base/desktop-app.md`.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::api::graph::flush_usage;
use crate::api::{OPENAPI_PATH, SWAGGER_UI_PATH};
use crate::index::sync::sync;
use crate::{AppState, Config, Index, Store, TimeStore, UsageTally, watcher};

/// How often API usage counts are moved from memory into the index.
const USAGE_FLUSH_INTERVAL: Duration = Duration::from_secs(60);

/// A running server, and the means to stop it.
///
/// Dropping one stops it too: every background task below watches the same
/// channel and reads its closure as a shutdown. That path skips the final usage
/// flush, so [`Server::shutdown`] is the one to prefer — but a dropped `Server`
/// that left a live listener and a file watcher behind would be worse.
pub struct Server {
    address: SocketAddr,
    state: AppState,
    /// Sending on this — or dropping it — stops everything below.
    halt: watch::Sender<bool>,
    serving: JoinHandle<std::io::Result<()>>,
    flusher: JoinHandle<()>,
    /// `None` when the wiki could not be watched, which is not fatal.
    watching: Option<JoinHandle<()>>,
}

impl Server {
    /// The address actually bound.
    ///
    /// Not necessarily `config.address`: port 0 means "whatever is free", and
    /// the answer is only knowable after the bind.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stop serving, then flush what has not been persisted yet.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        // A send error means every receiver is already gone, which is the state
        // being asked for rather than a problem.
        let _ = self.halt.send(true);

        // The server first, and awaited: graceful shutdown waits for requests
        // already in flight, and each of those still counts itself into the
        // tally that gets flushed at the bottom of this function.
        match self.serving.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => return Err(error).context("serving"),
            Err(error) => return Err(error).context("the server task"),
        }

        // Awaited rather than aborted. `abort` only *requests* cancellation, so
        // a periodic flush already in flight could still be running when the
        // final one below starts, and the two would split the last batch
        // between them.
        if let Err(error) = self.flusher.await {
            tracing::warn!(%error, "the usage flusher did not stop cleanly");
        }

        // The watcher is the one thing here that is cancelled outright. It
        // stops between batches when it can, but a reindex in progress is safe
        // to cut: it is idempotent and driven entirely by what is on disk, so
        // the worst case is one stale row that the next startup scan corrects.
        // Waiting for it is what releases its handle on the index.
        if let Some(watching) = self.watching {
            watching.abort();
            let _ = watching.await;
        }

        flush_usage(&self.state.index, &self.state.usage).await;

        tracing::info!("shut down");
        Ok(())
    }
}

/// Open the wiki, reconcile the index, and serve it.
///
/// Returns once the server is ready to answer requests.
pub async fn start(config: &Config) -> anyhow::Result<Server> {
    let store = Store::open(&config.root)
        .await
        .with_context(|| format!("opening the wiki at {}", config.root.display()))?;
    let times = TimeStore::open(&config.root)
        .await
        .context("opening the time log")?;
    let index = Index::open(Some(&config.database))
        .await
        .with_context(|| format!("opening the index at {}", config.database.display()))?;

    tracing::info!(wiki_root = %store.root_display(), "opened wiki");
    tracing::info!(time_log = %times.root_display(), "opened time log");

    reconcile(&store, &times, &index).await?;

    let listener = TcpListener::bind(config.address)
        .await
        .with_context(|| format!("binding {}", config.address))?;
    let address = listener.local_addr().context("reading the bound address")?;
    // ASCII only: the Windows console defaults to a codepage that mangles
    // anything else, and this is the first line anyone sees.
    tracing::info!("listening on http://{address}");
    tracing::info!("API docs at http://{address}{SWAGGER_UI_PATH}");
    tracing::info!("OpenAPI at http://{address}{OPENAPI_PATH}");

    // Started after the initial scan, so it only ever reports genuinely new
    // changes rather than racing the reconciliation that just ran.
    let watching = watcher::spawn(store.clone(), times.clone(), index.clone());

    let state = AppState {
        store,
        times,
        index,
        usage: UsageTally::new(),
        assets: assets(&config.assets).await,
    };

    let (halt, _) = watch::channel(false);

    let flusher = tokio::spawn(flush_usage_periodically(state.clone(), halt.subscribe()));

    let mut stopping = halt.subscribe();
    let router = crate::router(state.clone());
    let serving = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async move {
                // An error means the `Server` was dropped without a shutdown,
                // which stops us for the same reason a send would.
                let _ = stopping.changed().await;
            })
            .await
    });

    Ok(Server {
        address,
        state,
        halt,
        serving,
        flusher,
        watching,
    })
}

/// Bring the index in line with the files before anything is served.
///
/// The wiki may have been edited, or the whole index deleted, while the server
/// was down.
async fn reconcile(store: &Store, times: &TimeStore, index: &Index) -> anyhow::Result<()> {
    let report = sync(store, times, index)
        .await
        .context("reconciling the index with the wiki")?;

    tracing::info!(
        scanned = report.pages.scanned,
        indexed = report.pages.indexed,
        unchanged = report.pages.unchanged,
        removed = report.pages.removed,
        failed = report.pages.failed,
        "pages synchronised"
    );
    tracing::info!(
        scanned = report.times.scanned,
        indexed = report.times.indexed,
        unchanged = report.times.unchanged,
        removed = report.times.removed,
        failed = report.times.failed,
        "time log synchronised"
    );

    let failed = report.pages.failed + report.times.failed;
    if failed > 0 {
        tracing::warn!(
            failed,
            "some files could not be indexed; they will not appear in search or in the time log"
        );
    }

    Ok(())
}

/// The built frontend to serve, if there is one.
///
/// A missing build is normal during frontend development, when `pnpm dev`
/// serves the UI itself and proxies the API here.
async fn assets(path: &Path) -> Option<PathBuf> {
    match tokio::fs::try_exists(path).await {
        Ok(true) => {
            tracing::info!(path = %path.display(), "serving the built frontend");
            Some(path.to_path_buf())
        }
        _ => {
            tracing::info!(
                path = %path.display(),
                "no frontend build found; serving the API only"
            );
            None
        }
    }
}

async fn flush_usage_periodically(state: AppState, mut halt: watch::Receiver<bool>) {
    let mut ticker = tokio::time::interval(USAGE_FLUSH_INTERVAL);
    // The first tick fires immediately and would flush an empty tally.
    ticker.tick().await;

    loop {
        tokio::select! {
            _ = ticker.tick() => flush_usage(&state.index, &state.usage).await,
            _ = halt.changed() => break,
        }
    }
}
