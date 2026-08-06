use std::process::ExitCode;
use std::time::Duration;

use rhizolog::api::graph::flush_usage;
use rhizolog::api::{OPENAPI_PATH, SWAGGER_UI_PATH};
use rhizolog::index::sync;
use rhizolog::watcher;
use rhizolog::{AppState, Config, Index, Store, UsageTally};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

#[tokio::main]
async fn main() -> ExitCode {
    init_tracing();

    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let store = Store::open(&config.root).await?;
    let index = Index::open(Some(&config.database)).await?;

    tracing::info!(wiki_root = %store.root_display(), "opened wiki");

    // Reconcile before serving: the wiki may have been edited, or the whole
    // index deleted, while the server was down.
    let report = sync(&store, &index).await?;
    tracing::info!(
        scanned = report.scanned,
        indexed = report.indexed,
        unchanged = report.unchanged,
        removed = report.removed,
        failed = report.failed,
        "index synchronised"
    );
    if report.failed > 0 {
        tracing::warn!(
            failed = report.failed,
            "some pages could not be indexed; they will not appear in search"
        );
    }

    let listener = tokio::net::TcpListener::bind(config.address).await?;
    let address = listener.local_addr()?;
    // ASCII only: the Windows console defaults to a codepage that mangles
    // anything else, and this is the first line anyone sees.
    tracing::info!("listening on http://{address}");
    tracing::info!("API docs at http://{address}{SWAGGER_UI_PATH}");
    tracing::info!("OpenAPI at http://{address}{OPENAPI_PATH}");

    // Started after the initial scan, so it only ever reports genuinely new
    // changes rather than racing the reconciliation that just ran.
    watcher::spawn(store.clone(), index.clone());

    // A missing build is normal during frontend development, when `pnpm dev`
    // serves the UI itself and proxies the API here.
    let assets = match tokio::fs::try_exists(&config.assets).await {
        Ok(true) => {
            tracing::info!(path = %config.assets.display(), "serving the built frontend");
            Some(config.assets.clone())
        }
        _ => {
            tracing::info!(
                path = %config.assets.display(),
                "no frontend build found; serving the API only"
            );
            None
        }
    };

    let state = AppState {
        store,
        index,
        usage: UsageTally::new(),
        assets,
    };

    let flusher = tokio::spawn(flush_usage_periodically(state.clone()));

    let router = rhizolog::router(state.clone());
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    // Stop the periodic flush before the final one, so the two cannot race for
    // the tally and split the last batch between them.
    flusher.abort();
    flush_usage(&state.index, &state.usage).await;

    tracing::info!("shut down");
    Ok(())
}

/// How often API usage counts are moved from memory into the index.
const USAGE_FLUSH_INTERVAL: Duration = Duration::from_secs(60);

async fn flush_usage_periodically(state: AppState) {
    let mut ticker = tokio::time::interval(USAGE_FLUSH_INTERVAL);
    // The first tick fires immediately and would flush an empty tally.
    ticker.tick().await;

    loop {
        ticker.tick().await;
        flush_usage(&state.index, &state.usage).await;
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env(rhizolog::config::ENV_LOG)
        .unwrap_or_else(|_| EnvFilter::new("rhizolog=info,tower_http=info"));

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .init();
}

async fn shutdown_signal() {
    if let Err(error) = tokio::signal::ctrl_c().await {
        tracing::error!(%error, "failed to listen for shutdown signal");
        // Never returning leaves the server running rather than tearing it
        // down because we lost the ability to watch for Ctrl-C.
        std::future::pending::<()>().await;
    }
    tracing::info!("shutdown signal received");
}
