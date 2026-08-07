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

use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use tokio::net::TcpListener;
use tokio::sync::watch;
use tokio::task::JoinHandle;

use crate::api::graph::flush_usage;
use crate::api::{OPENAPI_PATH, SWAGGER_UI_PATH};
use crate::assets;
use crate::config::Listen;
use crate::endpoint::{self, Endpoint};
use crate::index::sync::sync;
use crate::store::display_path;
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
    /// Not necessarily the one that was asked for: a
    /// [`Preferably`](Listen::Preferably) address gives way when it is taken,
    /// and port 0 means "whatever is free". Either way the answer only exists
    /// after the bind, which is why it is published rather than assumed.
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// Stop serving, then flush what has not been persisted yet.
    pub async fn shutdown(self) -> anyhow::Result<()> {
        // A send error means every receiver is already gone, which is the state
        // being asked for rather than a problem.
        let _ = self.halt.send(true);

        // Withdrawn before anything is waited for, so the window in which the
        // file advertises a server that is going away is as small as it can be.
        if let Err(error) = endpoint::withdraw(self.state.store.root()).await {
            tracing::warn!(%error, "could not remove the endpoint file");
        }

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

    let listener = bind(config.listen).await?;
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
        assets: assets::resolve(&config.assets).await,
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

    // Last, and only now: the file exists exactly when there is a ready server
    // to find. See `crate::endpoint`.
    let root = state.store.root();
    if let Err(error) = endpoint::publish(root, &Endpoint::new(address, root)).await {
        // Discovery is a convenience. Refusing to serve a perfectly good wiki
        // because a hint about it could not be written would be the wrong
        // trade, so this is loud and not fatal.
        tracing::warn!(
            %error,
            path = %display_path(&endpoint::path(root)),
            "could not publish the endpoint file; callers will have to be told the address"
        );
    }

    Ok(Server {
        address,
        state,
        halt,
        serving,
        flusher,
        watching,
    })
}

/// Take the address asked for, or the nearest thing to it that is allowed.
///
/// The fallback is deliberately loud. A server that quietly moved is a server
/// somebody's bookmark no longer reaches, and the log is where they will look.
async fn bind(listen: Listen) -> anyhow::Result<TcpListener> {
    let address = listen.preferred();

    match TcpListener::bind(address).await {
        Ok(listener) => Ok(listener),
        Err(error) if error.kind() == ErrorKind::AddrInUse => match listen {
            Listen::Exactly(_) => Err(error).with_context(|| {
                format!("binding {address}: something else is already listening there")
            }),
            Listen::Preferably(_) => {
                tracing::warn!(
                    %address,
                    "that address is taken; falling back to any free port"
                );
                // Port 0: the OS picks. Same host, so a loopback-only default
                // cannot become a public one by falling back.
                let free = SocketAddr::new(address.ip(), 0);
                TcpListener::bind(free)
                    .await
                    .with_context(|| format!("binding {free} after {address} was taken"))
            }
        },
        Err(error) => Err(error).with_context(|| format!("binding {address}")),
    }
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
