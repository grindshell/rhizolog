//! The desktop app: a window onto a Rhizolog server running inside it.
//!
//! The server is not a detail this shell hides. It starts a real one on
//! loopback, in this process, and points a webview at it — so the dashboard in
//! the window is talking to the same HTTP API an agent would, and the app is
//! the same application whether it is looking at a wiki on this machine or one
//! somewhere else.
//!
//! Which is why this file is as small as it is. Its whole vocabulary is
//! [`Config`], [`Fallbacks`] and [`Server`]: work out which wiki, start one,
//! put a window in front of it, stop it again. It deliberately does not reach
//! for `Store`, `Index` or `TimeStore`, even though it can see them — a Tauri
//! command that read a page straight off disk would be a two-line convenience
//! and the end of the property above. When the shell needs wiki data, it asks
//! over HTTP like everything else.
//!
//! See `knowledge-base/desktop-app.md`.

// A GUI binary, so no console flashes behind the window. Debug builds keep one,
// because that is where `cargo run` output belongs.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod settings;

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rhizolog::{Config, Fallbacks, Server};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// The running server, so the way out can stop it.
#[derive(Default)]
struct Running(Mutex<Option<Server>>);

/// The id of the menu item that changes which wiki is open.
const OPEN_WIKI: &str = "open-wiki";

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(Running::default())
        .on_menu_event(|handle, event| {
            if event.id() == OPEN_WIKI {
                let handle = handle.clone();
                // The picker blocks, and this runs on the main thread.
                tauri::async_runtime::spawn(async move { open_another_wiki(handle).await });
            }
        })
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
    // `RHIZOLOG_ROOT` decides on its own when it is set, so there is nothing to
    // remember and nobody to ask — that is how the app gets pointed at a
    // scratch wiki without disturbing whatever it normally opens.
    let fallback_root = if environment_names_a_wiki() {
        None
    } else {
        match remembered_or_chosen(handle).await? {
            Some(root) => Some(root),
            // The picker was declined. Nothing to open, and nothing that needs
            // explaining in a dialog: they were asked, and they said no.
            None => {
                tracing::info!("no wiki chosen; exiting");
                handle.exit(0);
                return Ok(());
            }
        }
    };

    let config = Config::resolve(Fallbacks {
        root: fallback_root,
        assets: beside_executable("dist"),
        ..Fallbacks::default()
    })?;

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
        .menu(menu(handle)?)
        .build()?;

    tracing::info!(%url, wiki = %config.root.display(), "opened the window");
    Ok(())
}

/// Which wiki to open: what was chosen last time, or ask.
///
/// `None` means the picker was declined.
async fn remembered_or_chosen(handle: &AppHandle) -> anyhow::Result<Option<PathBuf>> {
    let mut stored = settings::load(handle);

    if let Some(root) = &stored.wiki_root {
        if root.is_dir() {
            return Ok(Some(root.clone()));
        }
        // Deliberately not passed through: `Store::open` creates its root, so
        // carrying on here would silently produce an empty wiki where somebody
        // else's used to be, and the first thing they would see is an empty
        // dashboard rather than a question.
        tracing::warn!(
            path = %root.display(),
            "the remembered wiki is not there any more; asking again"
        );
    }

    let Some(chosen) = pick_wiki(handle, "Choose a wiki folder").await else {
        return Ok(None);
    };

    stored.wiki_root = Some(chosen.clone());
    settings::save(handle, &stored)?;

    Ok(Some(chosen))
}

/// Change wikis, which means restarting.
///
/// `Store`, `TimeStore`, `Index` and the watcher are each bound to one root at
/// startup, so swapping wikis live means tearing all four down and standing
/// them back up. A restart does that correctly today; see
/// `knowledge-base/desktop-app.md`.
async fn open_another_wiki(handle: AppHandle) {
    let Some(chosen) = pick_wiki(&handle, "Open a different wiki").await else {
        return;
    };

    let mut stored = settings::load(&handle);
    if stored.wiki_root.as_ref() == Some(&chosen) {
        return;
    }

    stored.wiki_root = Some(chosen);
    if let Err(error) = settings::save(&handle, &stored) {
        tracing::error!("{error:#}");
        handle
            .dialog()
            .message(format!("Rhizolog could not save that choice.\n\n{error:#}"))
            .kind(MessageDialogKind::Error)
            .title("Rhizolog")
            .blocking_show();
        return;
    }

    // Stopped properly first: a restart that skipped this would lose the usage
    // counts and leave the endpoint file behind, describing a server that is
    // about to stop existing.
    stop(&handle).await;
    handle.restart();
}

async fn pick_wiki(handle: &AppHandle, title: &str) -> Option<PathBuf> {
    let start_in = handle
        .path()
        .document_dir()
        .or_else(|_| handle.path().home_dir())
        .ok();

    let mut picker = handle.dialog().file().set_title(title);
    if let Some(directory) = start_in {
        picker = picker.set_directory(directory);
    }

    let chosen = picker.blocking_pick_folder()?;
    match chosen.into_path() {
        Ok(path) => Some(path),
        Err(error) => {
            tracing::error!(%error, "the chosen folder is not a path this can use");
            None
        }
    }
}

/// The window's menu.
///
/// Native conveniences belong here rather than in the page: the dashboard has
/// to stay identical to the one a browser loads from a remote instance, and
/// anything it could only do inside this window would break that.
fn menu(handle: &AppHandle) -> tauri::Result<Menu<tauri::Wry>> {
    // Nothing to change when `RHIZOLOG_ROOT` is set: the app is not the thing
    // deciding, and a restart would come straight back to the same wiki.
    let open = MenuItem::with_id(
        handle,
        OPEN_WIKI,
        "Open Wiki…",
        !environment_names_a_wiki(),
        None::<&str>,
    )?;
    let quit = PredefinedMenuItem::quit(handle, None)?;
    let file = Submenu::with_items(handle, "File", true, &[&open, &quit])?;

    Menu::with_items(handle, &[&file])
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

fn environment_names_a_wiki() -> bool {
    std::env::var_os(rhizolog::config::ENV_ROOT).is_some()
}

/// A path next to the executable.
///
/// The working directory is not something a double-clicked application gets to
/// know: Explorer sets it to the executable's directory, a shortcut sets it to
/// whatever its "Start in" says, and a file association sets it to the opened
/// file's folder. Anything a portable copy carries with it has to be found
/// relative to the executable instead.
fn beside_executable(name: &str) -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|executable| executable.parent().map(|directory| directory.join(name)))
        .unwrap_or_else(|| PathBuf::from(name))
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
