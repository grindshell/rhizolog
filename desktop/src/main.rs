//! The desktop app: a window onto a Rhizolog server running inside it.
//!
//! The server is not a detail this shell hides. It starts a real one on
//! loopback, in this process, and points a webview at it — so the dashboard in
//! the window is talking to the same HTTP API an agent would, and the app is
//! the same application whether it is looking at a wiki on this machine or one
//! somewhere else.
//!
//! Which is why this file is as small as it is. Its whole vocabulary is
//! [`Config`] and [`Server`]: start one, put a window in front of it, stop it
//! again. It deliberately does not reach for `Store`, `Index` or `TimeStore`,
//! even though it can see them — a Tauri command that read a page straight off
//! disk would be a two-line convenience and the end of the property above. When
//! the shell needs wiki data, it asks over HTTP like everything else.
//!
//! See `knowledge-base/desktop-app.md`.

// A GUI binary, so no console flashes behind the window. Debug builds keep one,
// because that is where `cargo run` output belongs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rhizolog::{Config, Server};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// The running server, so the way out can stop it.
#[derive(Default)]
struct Running(Mutex<Option<Server>>);

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Running::default())
        .setup(|app| {
            init_tracing(app.handle());

            // Spawned rather than awaited: the event loop should be running
            // while the wiki is reconciled, or a large one would look like a
            // launch that did nothing. The window appears when there is
            // something behind it.
            let handle = app.handle().clone();
            tauri::async_runtime::spawn(async move { launch(handle).await });

            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("failed to build the application");

    // Closing the window is not the end of the process's obligations: the usage
    // tally has to be flushed and the index let go of. `prevent_exit` buys the
    // time to do it, and the flag stops the `exit` at the end of that work
    // coming back round as another request to shut down.
    let stopping = AtomicBool::new(false);

    app.run(move |handle, event| {
        if let RunEvent::ExitRequested { api, .. } = event {
            if stopping.swap(true, Ordering::SeqCst) {
                return;
            }

            api.prevent_exit();
            let handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                stop(&handle).await;
                handle.exit(0);
            });
        }
    });
}

/// Start the server and show it, or explain why not.
async fn launch(handle: AppHandle) {
    let Err(error) = start(&handle).await else {
        return;
    };

    tracing::error!("{error:#}");

    // There is no window yet and nothing to put in one, so this is the only
    // place the failure can be said out loud. A GUI binary that exits silently
    // looks like a launch that did not happen.
    handle
        .dialog()
        .message(format!("Rhizolog could not start.\n\n{error:#}"))
        .kind(MessageDialogKind::Error)
        .title("Rhizolog")
        .blocking_show();

    handle.exit(1);
}

async fn start(handle: &AppHandle) -> anyhow::Result<()> {
    let config = Config::from_env()?;
    let server = rhizolog::server::start(&config).await?;

    // The address is only knowable after the bind, which is why the window is
    // built here rather than declared in `tauri.conf.json`.
    let url = Url::parse(&format!("http://{}", server.address()))?;
    handle
        .state::<Running>()
        .0
        .lock()
        .expect("the server lock")
        .replace(server);

    WebviewWindowBuilder::new(handle, "main", WebviewUrl::External(url.clone()))
        .title("Rhizolog")
        .inner_size(1280.0, 860.0)
        .min_inner_size(720.0, 480.0)
        .build()?;

    tracing::info!(%url, "opened the window");
    Ok(())
}

async fn stop(handle: &AppHandle) {
    // Taken out of the lock before anything is awaited, so the guard is not
    // held across a shutdown that waits on the server, the flusher and the
    // watcher in turn.
    let server = {
        let running = handle.state::<Running>();
        let mut slot = running.0.lock().expect("the server lock");
        slot.take()
    };

    let Some(server) = server else {
        return;
    };

    if let Err(error) = server.shutdown().await {
        tracing::error!("{error:#}");
    }
}

/// Log to a file, because a windowed binary has no stdout to log to.
///
/// `RHIZOLOG_LOG` still filters, and debug builds still print — but a release
/// build's only account of itself is this file, so a launch that fails before
/// there is a window has to leave something behind.
fn init_tracing(handle: &AppHandle) {
    let filter = EnvFilter::try_from_env(rhizolog::config::ENV_LOG)
        .unwrap_or_else(|_| EnvFilter::new("rhizolog=info,rhizolog_desktop=info"));

    let file = handle.path().app_log_dir().ok().and_then(|directory| {
        std::fs::create_dir_all(&directory).ok()?;
        Some(tracing_appender::rolling::daily(&directory, "rhizolog.log"))
    });

    tracing_subscriber::registry()
        .with(filter)
        .with(tracing_subscriber::fmt::layer())
        .with(file.map(|file| {
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file)
        }))
        .init();
}
