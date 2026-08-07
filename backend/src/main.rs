//! The headless server.
//!
//! Everything this does beyond reading the environment and waiting for Ctrl-C
//! is [`rhizolog::server`], so that a shell with a different idea of when to
//! stop — a desktop window, a test — starts the same server this does.

use std::process::ExitCode;

use rhizolog::Config;
use rhizolog::server;
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
    let server = server::start(&config).await?;

    shutdown_signal().await;
    server.shutdown().await
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
