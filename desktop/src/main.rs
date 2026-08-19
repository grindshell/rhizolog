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
mod settings_window;

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

use rhizolog::config::DEFAULT_ADDRESS;
use rhizolog::{Config, Fallbacks, Listen, Server};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::webview::{NewWindowFeatures, NewWindowResponse};
use tauri::{AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, Wry};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tauri_plugin_opener::OpenerExt;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::prelude::*;

/// The running server, so the way out can stop it.
#[derive(Default)]
struct Running(Mutex<Option<Server>>);

/// The id of the menu item that changes which wiki is open.
const OPEN_WIKI: &str = "open-wiki";

/// The id of the menu item that opens the settings window.
const SETTINGS: &str = "settings";

fn main() {
    let app = tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        // Only ever called from Rust, by [`open_elsewhere`]. The plugin's own
        // answer to `target="_blank"` is a script it injects into the page,
        // which cancels the click and then calls Tauri IPC — and this page is a
        // remote origin with no capability, so it would swap one link that does
        // nothing for another. Off, and the shell answers instead.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        // The settings window's own page, which is the shell's and not the
        // wiki's. A scheme rather than a route on the server: the dashboard has
        // to be the same page a browser loads from a remote instance, and this
        // is a control only a local app can honour.
        .register_uri_scheme_protocol(settings_window::SCHEME, |context, request| {
            settings_window::respond(context.app_handle(), context.webview_label(), &request)
        })
        .manage(Running::default())
        .on_menu_event(|handle, event| {
            if event.id() == OPEN_WIKI {
                let handle = handle.clone();
                // The picker blocks, and this runs on the main thread.
                tauri::async_runtime::spawn(async move { open_another_wiki(handle).await });
            } else if event.id() == SETTINGS
                && let Err(error) = settings_window::open(handle)
            {
                tracing::error!(%error, "could not open the settings window");
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
    // Read once, and before the wiki question: the port is remembered whether
    // or not `RHIZOLOG_ROOT` answered that one, and the two settings have
    // nothing to do with each other.
    let stored = settings::load(handle);

    // `RHIZOLOG_ROOT` decides on its own when it is set, so there is nothing to
    // remember and nobody to ask — that is how the app gets pointed at a
    // scratch wiki without disturbing whatever it normally opens.
    let mut fallback_root = if environment_names_a_wiki() {
        None
    } else {
        match remembered_or_chosen(handle, &stored).await? {
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

    // One instance per wiki. Not one per application: two windows on two
    // different wikis is a perfectly reasonable thing to want, and two on the
    // same one means two writers on an index, two file watchers, and an
    // endpoint file that can only describe the newer of them.
    let mut config = loop {
        let config = Config::resolve(Fallbacks {
            root: fallback_root.clone(),
            assets: beside_executable("dist"),
            listen: listen_for(stored.port),
        })?;

        let Some(existing) = rhizolog::endpoint::live(&config.root).await else {
            break config;
        };

        tracing::info!(
            url = %existing.url,
            wiki = %config.root.display(),
            "this wiki is already open"
        );

        let Some(instead) = already_open(handle, &config, &existing).await else {
            handle.exit(0);
            return Ok(());
        };

        remember(handle, &instead)?;
        fallback_root = Some(instead);
    };

    let server = match rhizolog::server::start(&config).await {
        Ok(server) => server,
        // A port somebody chose in the settings window is `Exactly`, so a
        // machine that has since acquired something else on it is an app that
        // will not start — every launch, with no way in to change the setting,
        // because the way in is a menu on a window that never appears.
        Err(error) if a_chosen_port_is_taken(&config, &error) => {
            tracing::warn!("{error:#}");

            if !offer_an_automatic_port(handle, config.listen.preferred().port()).await {
                handle.exit(0);
                return Ok(());
            }

            forget_the_port(handle)?;
            config.listen = Listen::Preferably(DEFAULT_ADDRESS);
            rhizolog::server::start(&config).await?
        }
        Err(error) => return Err(error),
    };

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
        .on_new_window(link_handler(handle, &url))
        .build()?;

    tracing::info!(%url, wiki = %config.root.display(), "opened the window");
    Ok(())
}

/// Which wiki to open: what was chosen last time, or ask.
///
/// `None` means the picker was declined.
async fn remembered_or_chosen(
    handle: &AppHandle,
    stored: &settings::Settings,
) -> anyhow::Result<Option<PathBuf>> {
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

    remember(handle, &chosen)?;
    Ok(Some(chosen))
}

/// Say that this wiki is already open, and offer the way out.
///
/// `None` means quit. Something else means open that instead.
async fn already_open(
    handle: &AppHandle,
    config: &Config,
    existing: &rhizolog::Endpoint,
) -> Option<PathBuf> {
    let message = format!(
        "Rhizolog is already open on this wiki.\n\n{}\n\nThat window is serving {}.",
        config.root.display(),
        existing.url,
    );

    // `RHIZOLOG_ROOT` chose this wiki, so offering a different one would be
    // offering something this launch has no way to honour.
    if environment_names_a_wiki() {
        handle
            .dialog()
            .message(message)
            .kind(MessageDialogKind::Warning)
            .title("Rhizolog")
            .blocking_show();
        return None;
    }

    let choose_another = handle
        .dialog()
        .message(format!(
            "{message}\n\nSwitch to that window, or open a different wiki here."
        ))
        .kind(MessageDialogKind::Warning)
        .title("Rhizolog")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Open a different wiki…".to_owned(),
            "Quit".to_owned(),
        ))
        .blocking_show();

    if !choose_another {
        return None;
    }

    pick_wiki(handle, "Open a different wiki").await
}

/// Write down which wiki to open next time.
fn remember(handle: &AppHandle, root: &Path) -> anyhow::Result<()> {
    let mut stored = settings::load(handle);
    stored.wiki_root = Some(root.to_path_buf());
    settings::save(handle, &stored)
}

/// What to bind, for whatever the environment did not say.
///
/// A port from the settings window is [`Listen::Exactly`], for the same reason
/// `RHIZOLOG_ADDR` is: somebody who chose a port chose it *for* something —
/// a bookmark, a proxy, an agent's configuration — and a server that quietly
/// started somewhere else would point that something at nothing. `None` is the
/// ordinary case and stays a preference.
///
/// Only the port is theirs to choose. Loopback is the entire security boundary,
/// and it is also what keeps a wiki from asking for a firewall exception the
/// first time it runs.
fn listen_for(port: Option<u16>) -> Listen {
    match port {
        Some(port) => Listen::Exactly(SocketAddr::new(DEFAULT_ADDRESS.ip(), port)),
        None => Listen::Preferably(DEFAULT_ADDRESS),
    }
}

/// Whether this is a port the app chose and could stop choosing.
///
/// `RHIZOLOG_ADDR` produces the same [`Listen::Exactly`] and is not the app's
/// to withdraw: somebody set it deliberately, in a shell, and the right answer
/// there is the error.
fn a_chosen_port_is_taken(config: &Config, error: &anyhow::Error) -> bool {
    matches!(config.listen, Listen::Exactly(_))
        && !environment_names_an_address()
        && error.chain().any(|cause| {
            cause
                .downcast_ref::<std::io::Error>()
                .is_some_and(|io| io.kind() == std::io::ErrorKind::AddrInUse)
        })
}

/// Offer to stop insisting on a port that is not available.
///
/// `true` to carry on with a negotiated one.
async fn offer_an_automatic_port(handle: &AppHandle, port: u16) -> bool {
    handle
        .dialog()
        .message(format!(
            "Rhizolog is set to serve on port {port}, and something else on this machine is \
             already using it.\n\nRhizolog can forget that setting and take any free port \
             instead."
        ))
        .kind(MessageDialogKind::Warning)
        .title("Rhizolog")
        .buttons(MessageDialogButtons::OkCancelCustom(
            "Use any free port".to_owned(),
            "Quit".to_owned(),
        ))
        .blocking_show()
}

/// Stop asking for a particular port.
fn forget_the_port(handle: &AppHandle) -> anyhow::Result<()> {
    let mut stored = settings::load(handle);
    stored.port = None;
    settings::save(handle, &stored)
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

    if settings::load(&handle).wiki_root.as_ref() == Some(&chosen) {
        return;
    }

    if let Err(error) = remember(&handle, &chosen) {
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

/// The shell's answer to `target="_blank"`, ready to hand to a window builder.
///
/// Every window gets one, the ones opened by [`new_window`] included: a second
/// window is a browser with no tabs for the same reason the first one is, and
/// Swagger UI's own links out are as dead there as the dashboard's were here.
///
/// It closes over the server's address, which is only knowable once the bind
/// has happened — the same reason the window is built in `start` rather than
/// declared in `tauri.conf.json`.
fn link_handler(
    handle: &AppHandle,
    ours: &Url,
) -> impl Fn(Url, NewWindowFeatures) -> NewWindowResponse<Wry> + Send + 'static {
    let handle = handle.clone();
    let ours = ours.clone();

    move |requested, features| new_window(&handle, &ours, requested, features)
}

/// Answer a `target="_blank"`.
///
/// A webview is a browser with no tabs, and WebView2's default for a new-window
/// request that nothing is listening for is to mark it handled and drop it —
/// so the dashboard's "API docs" link, and every external link in a page body,
/// silently does nothing when clicked. Neither is an exotic request; a browser
/// would honour both, and the app has to look like one.
///
/// This wiki's own pages get a second window onto the same server. Everything
/// else is the open web and goes to the real browser: a Tauri window has no
/// address bar, no back button and nothing that says whose site is in it.
fn new_window(
    handle: &AppHandle,
    ours: &Url,
    requested: Url,
    features: NewWindowFeatures,
) -> NewWindowResponse<Wry> {
    if !is_ours(ours, &requested) {
        open_elsewhere(handle, &requested);
        return NewWindowResponse::Deny;
    }

    // A second click on the same link should come back to the window the first
    // one opened, not stack another identical one on top of it.
    let label = label_for(&requested);
    if let Some(already) = handle.get_webview_window(&label) {
        let _ = already.unminimize();
        let _ = already.set_focus();
        return NewWindowResponse::Deny;
    }

    // `window_features` is not decoration: on Windows a webview made for a
    // new-window request has to share the opener's WebView2 environment, and
    // that is what carries it.
    //
    // No menu, deliberately. File → Open Wiki… restarts the application, which
    // is not a thing to offer from a window looking at one page.
    let built = WebviewWindowBuilder::new(handle, &label, WebviewUrl::External(requested.clone()))
        .title("Rhizolog")
        .inner_size(1100.0, 820.0)
        .min_inner_size(480.0, 360.0)
        .window_features(features)
        .on_new_window(link_handler(handle, ours))
        // Two windows both called "Rhizolog" say nothing about which is which,
        // so this one is named by whatever it ends up showing.
        .on_document_title_changed(|window, title| {
            let _ = window.set_title(&title);
        })
        .build();

    match built {
        Ok(window) => NewWindowResponse::Create { window },
        Err(error) => {
            tracing::error!(url = %requested, %error, "could not open a second window");
            NewWindowResponse::Deny
        }
    }
}

/// Whether a URL is a page of the server this window is showing.
///
/// The origin, so the port is part of the answer — two instances on two wikis
/// are both `127.0.0.1`, and the other one's pages are no more this window's
/// business than any other site is.
fn is_ours(ours: &Url, requested: &Url) -> bool {
    requested.origin() == ours.origin()
}

/// Hand a link to whatever the machine opens links with.
///
/// The scheme list is the check, not a nicety. A wiki page body can carry any
/// URL it likes, and this ends at `ShellExecute` on Windows — so `file:` and
/// anything else that names a program stops here rather than being launched by
/// a click on a wiki page.
fn open_elsewhere(handle: &AppHandle, url: &Url) {
    if !matches!(url.scheme(), "http" | "https" | "mailto" | "tel") {
        tracing::warn!(%url, "not opening a link with that scheme");
        return;
    }

    if let Err(error) = handle.opener().open_url(url.as_str(), None::<&str>) {
        tracing::error!(%url, %error, "could not hand the link to a browser");
    }
}

/// The label of the window showing a page of this server.
///
/// Labels have to be unique, and this one is also how the next click finds the
/// window the last one opened — so it names the page rather than the click.
/// Tauri accepts alphanumerics and a little punctuation, which a URL path does
/// not promise, so everything else becomes a dash.
fn label_for(url: &Url) -> String {
    let path: String = url
        .path()
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character
            } else {
                '-'
            }
        })
        .collect();

    format!("view{path}")
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
    // Always enabled, even when the environment has decided the address: the
    // window is where "this box does nothing, and here is what is overriding
    // it" gets said, and a greyed-out menu item cannot say it.
    let settings = MenuItem::with_id(handle, SETTINGS, "Settings…", true, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(handle)?;
    let quit = PredefinedMenuItem::quit(handle, None)?;
    let file = Submenu::with_items(handle, "File", true, &[&open, &settings, &separator, &quit])?;

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

fn environment_names_an_address() -> bool {
    std::env::var_os(rhizolog::config::ENV_ADDRESS).is_some()
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

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).expect("a url")
    }

    /// The link that started this: the dashboard's "API docs" is a plain
    /// `target="_blank"` anchor to a path on the very server the window is
    /// already showing, so it must be recognised as ours and not posted off to
    /// a browser.
    #[test]
    fn a_page_of_this_server_is_ours() {
        let ours = url("http://127.0.0.1:3000/");

        assert!(is_ours(&ours, &url("http://127.0.0.1:3000/swagger-ui")));
        assert!(is_ours(&ours, &url("http://127.0.0.1:3000/api/health")));
    }

    /// The port is negotiated, so two instances on two wikis differ only there.
    /// The other one's pages are somebody else's window's business.
    #[test]
    fn another_instance_is_not_ours() {
        assert!(!is_ours(
            &url("http://127.0.0.1:3000/"),
            &url("http://127.0.0.1:49812/swagger-ui")
        ));
    }

    #[test]
    fn the_web_is_not_ours() {
        let ours = url("http://127.0.0.1:3000/");

        assert!(!is_ours(&ours, &url("https://v2.tauri.app/")));
        assert!(!is_ours(&ours, &url("file:///C:/Windows/System32/")));
        assert!(!is_ours(&ours, &url("about:blank")));
    }

    /// Tauri rejects a label with punctuation in it, and a URL path is mostly
    /// punctuation. A label that cannot be built is a window that never opens.
    #[test]
    fn a_label_is_alphanumerics_and_dashes() {
        for path in [
            "/swagger-ui",
            "/",
            "/pages/notes/a page.md",
            "/pages/tags?q=a%20b#top",
        ] {
            let label = label_for(&url(&format!("http://127.0.0.1:3000{path}")));
            assert!(
                label
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || character == '-'),
                "{path} gave {label}"
            );
        }
    }

    /// The distinction `Listen` exists for, arriving from a third place. A
    /// remembered port was typed by somebody who meant it; the absence of one
    /// is a preference the server may give way on.
    #[test]
    fn a_remembered_port_is_binding_and_no_port_is_not() {
        assert_eq!(
            listen_for(Some(8080)),
            Listen::Exactly("127.0.0.1:8080".parse().expect("an address"))
        );
        assert_eq!(listen_for(None), Listen::Preferably(DEFAULT_ADDRESS));
    }

    /// The host is not part of the choice. Binding anything but loopback would
    /// put an unauthenticated API that writes files on the network, and raise a
    /// firewall prompt on the way.
    #[test]
    fn a_remembered_port_stays_on_loopback() {
        assert!(listen_for(Some(8080)).preferred().ip().is_loopback());
    }

    /// Two pages must not share a window, and nothing may collide with `main` —
    /// building a window with a label already in use fails, and the failure
    /// would look exactly like the bug this all fixes.
    #[test]
    fn a_label_names_the_page() {
        let swagger = label_for(&url("http://127.0.0.1:3000/swagger-ui"));

        assert_eq!(
            swagger,
            label_for(&url("http://127.0.0.1:3000/swagger-ui")),
            "the same page twice is the same window"
        );
        assert_ne!(swagger, label_for(&url("http://127.0.0.1:3000/api/health")));
        assert_ne!(swagger, "main");
    }
}
