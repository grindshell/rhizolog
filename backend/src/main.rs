use std::process::ExitCode;

use rhizowiki::api::{OPENAPI_PATH, SWAGGER_UI_PATH};
use rhizowiki::index::sync;
use rhizowiki::{AppState, Config, Index, Store};
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

    let router = rhizowiki::router(AppState { store, index });
    axum::serve(listener, router)
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    tracing::info!("shut down");
    Ok(())
}

fn init_tracing() {
    let filter = EnvFilter::try_from_env(rhizowiki::config::ENV_LOG)
        .unwrap_or_else(|_| EnvFilter::new("rhizowiki=info,tower_http=info"));

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
