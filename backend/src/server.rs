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
use crate::users::UserStore;
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
    let users = UserStore::open(&config.root)
        .await
        .context("opening the accounts directory")?;
    let index = Index::open(Some(&config.database))
        .await
        .with_context(|| format!("opening the index at {}", config.database.display()))?;

    tracing::info!(wiki_root = %store.root_display(), "opened wiki");
    tracing::info!(time_log = %times.root_display(), "opened time log");

    reconcile(&store, &times, &index).await?;
    announce_access(&users, config).await?;

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
        users,
        index,
        usage: UsageTally::new(),
        assets: assets::resolve(&config.assets).await,
        secure_cookies: config.secure_cookies,
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

/// What this server will answer to, and whether that is alarming.
///
/// Separated from the logging so it can be tested. The alarming combination is
/// the one nobody assembles on purpose: it takes an address set in one place and
/// an account never created in another, and those two acts are far enough apart
/// in time that startup is the only moment they meet.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Access {
    /// No accounts, bound to loopback. The ordinary local wiki.
    Open,
    /// No accounts, reachable from the network. Anybody who can find it can
    /// read and write every page in it.
    OpenToTheNetwork,
    /// Accounts, and either loopback or TLS in front.
    Authenticated,
    /// Accounts, off loopback, over plain HTTP — so passwords and session
    /// tokens cross the network in the clear.
    AuthenticatedInTheClear,
}

impl Access {
    pub(crate) fn of(accounts: usize, config: &Config) -> Self {
        let exposed = !config.listen.preferred().ip().is_loopback();

        match (accounts > 0, exposed) {
            (false, false) => Self::Open,
            (false, true) => Self::OpenToTheNetwork,
            // `secure_cookies` is the closest thing the server has to being told
            // there is TLS in front of it. It is not proof, but somebody who set
            // it has thought about the question, and warning them anyway would
            // train them to ignore the line.
            (true, true) if !config.secure_cookies => Self::AuthenticatedInTheClear,
            (true, _) => Self::Authenticated,
        }
    }

    /// The line worth logging on every start.
    pub(crate) fn summary(self) -> &'static str {
        match self {
            Self::Open | Self::OpenToTheNetwork => {
                "this wiki has no accounts; every request is the single user"
            }
            Self::Authenticated | Self::AuthenticatedInTheClear => {
                "this wiki requires authentication"
            }
        }
    }

    /// What to say loudly, when there is anything.
    pub(crate) fn warning(self) -> Option<&'static str> {
        match self {
            Self::Open | Self::Authenticated => None,
            Self::OpenToTheNetwork => Some(
                "this wiki has no accounts and is not bound to loopback: anybody who can reach \
                 this address can read and write every page. Create an account to require a \
                 sign-in.",
            ),
            Self::AuthenticatedInTheClear => Some(
                "serving off loopback over plain HTTP: passwords and session tokens cross the \
                 network in the clear. Put a TLS proxy in front and set \
                 RHIZOLOG_SECURE_COOKIES=1.",
            ),
        }
    }
}

/// Say, once, who this server will answer to.
///
/// Worth a line in the log on every start, because the answer is a property of
/// the *wiki directory* rather than of anything in the configuration — there is
/// no flag to read back, and "does this instance require a sign-in" is otherwise
/// only discoverable by trying it.
///
/// The warnings stay warnings rather than refusals: a deliberately open instance
/// behind a firewall is a legitimate thing to run. Nobody should arrive at one by
/// accident, which is a different problem and the one a log line solves.
async fn announce_access(users: &UserStore, config: &Config) -> anyhow::Result<()> {
    let accounts = users
        .count()
        .await
        .context("counting the accounts in the wiki")?;

    let access = Access::of(accounts, config);
    tracing::info!(accounts, "{}", access.summary());

    if let Some(warning) = access.warning() {
        tracing::warn!(address = %config.listen.preferred(), "{warning}");
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

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn config(address: &str, secure_cookies: bool) -> Config {
        Config {
            root: PathBuf::from("/wiki"),
            database: PathBuf::from("/wiki/.rhizolog/index.db"),
            listen: Listen::Exactly(address.parse().expect("an address")),
            assets: PathBuf::from("/wiki/dist"),
            secure_cookies,
        }
    }

    /// The default, and the state the desktop app spends its life in.
    #[test]
    fn no_accounts_on_loopback_is_the_ordinary_wiki() {
        let access = Access::of(0, &config("127.0.0.1:3000", false));

        assert_eq!(access, Access::Open);
        assert_eq!(access.warning(), None);
    }

    /// The combination nobody assembles on purpose: an unauthenticated API that
    /// writes files, reachable from the network.
    #[test]
    fn no_accounts_off_loopback_is_the_one_worth_shouting_about() {
        for address in ["0.0.0.0:3000", "192.168.1.10:3000", "[::]:3000"] {
            let access = Access::of(0, &config(address, false));

            assert_eq!(access, Access::OpenToTheNetwork, "for {address}");
            assert!(access.warning().is_some(), "for {address}");
        }
    }

    /// Secure cookies do not make an open wiki safe. There is nothing to
    /// protect a session for when there are no sessions.
    #[test]
    fn tls_does_not_excuse_an_open_wiki() {
        assert_eq!(
            Access::of(0, &config("0.0.0.0:3000", true)),
            Access::OpenToTheNetwork
        );
    }

    #[test]
    fn accounts_on_loopback_are_quiet() {
        let access = Access::of(1, &config("127.0.0.1:3000", false));

        assert_eq!(access, Access::Authenticated);
        assert_eq!(access.warning(), None);
    }

    /// Passwords and session tokens crossing a network in the clear.
    #[test]
    fn accounts_off_loopback_over_plain_http_is_alarming() {
        let access = Access::of(2, &config("0.0.0.0:3000", false));

        assert_eq!(access, Access::AuthenticatedInTheClear);
        assert!(access.warning().is_some());
    }

    /// Somebody who set the flag has thought about the question. Warning anyway
    /// would train them to ignore the line.
    #[test]
    fn saying_there_is_tls_in_front_settles_it() {
        let access = Access::of(2, &config("0.0.0.0:3000", true));

        assert_eq!(access, Access::Authenticated);
        assert_eq!(access.warning(), None);
    }

    /// IPv6 loopback is loopback.
    #[test]
    fn ipv6_loopback_counts_as_loopback() {
        assert_eq!(Access::of(0, &config("[::1]:3000", false)), Access::Open);
    }
}
