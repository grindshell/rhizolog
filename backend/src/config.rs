//! Environment-driven configuration.

use std::env;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use thiserror::Error;

use crate::store::INTERNAL_DIR;

pub const ENV_ROOT: &str = "RHIZOLOG_ROOT";
pub const ENV_DATABASE: &str = "RHIZOLOG_DB";
pub const ENV_ADDRESS: &str = "RHIZOLOG_ADDR";
pub const ENV_ASSETS: &str = "RHIZOLOG_ASSETS";
pub const ENV_LOG: &str = "RHIZOLOG_LOG";
pub const ENV_SECURE_COOKIES: &str = "RHIZOLOG_SECURE_COOKIES";

/// Loopback, deliberately.
///
/// It used to be the *whole* security boundary, on the grounds that Rhizolog was
/// single-user and had no authentication. Accounts change what that sentence
/// means without changing the default: a wiki with no accounts is still open and
/// still writes files from any request, so putting one on a network by default
/// would be exactly as wrong as it ever was. Binding wider is now a supported
/// thing to *choose*, once there is an account to sign in to — see
/// `knowledge-base/accounts.md`.
pub const DEFAULT_ADDRESS: SocketAddr =
    SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), DEFAULT_PORT);

/// Worth having a memorable one. It is what the README, the Vite proxy and
/// anybody's notes all assume.
pub const DEFAULT_PORT: u16 = 3000;

/// What to bind, and how hard to insist on it.
///
/// The two ways an address arrives mean different things. A default is a
/// preference, and a preference that cannot be met should give way rather than
/// stop the server starting at all — a second copy of the app finding 3000
/// taken is an ordinary Tuesday. An address somebody wrote down is a
/// requirement: they wrote it down somewhere else too, and quietly serving on a
/// different port would point that somewhere else at nothing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Listen {
    /// This address or nothing.
    Exactly(SocketAddr),
    /// This address if it is free, otherwise any free port on the same host.
    Preferably(SocketAddr),
}

impl Listen {
    /// The address that gets tried first, whichever kind this is.
    pub fn preferred(self) -> SocketAddr {
        match self {
            Self::Exactly(address) | Self::Preferably(address) => address,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The wiki directory. Markdown files here are the source of truth.
    pub root: PathBuf,
    /// The derived SQLite index. Safe to delete; it rebuilds on startup.
    pub database: PathBuf,
    pub listen: Listen,
    /// The built frontend to serve.
    ///
    /// Missing is a normal state, not an error: during frontend development
    /// `pnpm dev` serves the UI itself and proxies the API here, so nothing has
    /// been built yet.
    pub assets: PathBuf,
    /// Whether to mark the session cookie `Secure`.
    ///
    /// Off by default, and it has to be: the server speaks HTTP, and a browser
    /// discards a `Secure` cookie that arrives over one. The symptom is a
    /// sign-in that returns 200 and leaves you signed out, which is a horrible
    /// thing to debug.
    ///
    /// Set `RHIZOLOG_SECURE_COOKIES=1` when a TLS-terminating proxy sits in
    /// front. It is a separate switch rather than something inferred from
    /// `X-Forwarded-Proto` because inferring it means trusting a header that
    /// anybody who can reach the port can send.
    pub secure_cookies: bool,
}

/// What to use for anything the environment did not say.
///
/// The server's answers are all relative to the working directory, which is
/// right for something started from a shell and meaningless for something
/// started by a double-click — a GUI has no shell environment behind it, and
/// its working directory is whatever Explorer felt like. So the caller supplies
/// these rather than `Config` assuming them, and each binary gets to be right
/// about its own situation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fallbacks {
    /// The wiki to open when `RHIZOLOG_ROOT` is not set.
    ///
    /// `None` says the caller has none to offer, which is only a safe thing to
    /// say when the variable is set — the desktop app, which asks the user
    /// instead, has nothing to put here when the environment has already
    /// decided. Getting that wrong is [`ConfigError::NoWikiRoot`] rather than a
    /// wiki quietly created somewhere nobody will look.
    pub root: Option<PathBuf>,
    pub assets: PathBuf,
    pub listen: Listen,
}

impl Default for Fallbacks {
    /// What the headless server uses: relative to the working directory, which
    /// is the checkout it was started from.
    fn default() -> Self {
        Self {
            root: Some(PathBuf::from("./wiki")),
            // Relative to the repository root, which is where `cargo run` is
            // usually invoked from via `backend/`.
            assets: PathBuf::from("../frontend/dist"),
            listen: Listen::Preferably(DEFAULT_ADDRESS),
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{ENV_ADDRESS} is not a valid socket address: {value:?}")]
    Address {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
    #[error("no wiki directory: set {ENV_ROOT}, or choose one")]
    NoWikiRoot,
}

/// The values the environment supplied, which are all optional and all win.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Overrides {
    root: Option<PathBuf>,
    database: Option<PathBuf>,
    address: Option<String>,
    assets: Option<PathBuf>,
    secure_cookies: Option<bool>,
}

impl Overrides {
    fn from_env() -> Self {
        Self {
            root: env::var_os(ENV_ROOT).map(PathBuf::from),
            database: env::var_os(ENV_DATABASE).map(PathBuf::from),
            address: env::var(ENV_ADDRESS).ok(),
            assets: env::var_os(ENV_ASSETS).map(PathBuf::from),
            secure_cookies: env::var(ENV_SECURE_COOKIES)
                .ok()
                .map(|value| truthy(&value)),
        }
    }
}

/// What counts as "on" for a boolean environment variable.
///
/// Generous on purpose: somebody setting this is doing it in a shell, a
/// Dockerfile or a systemd unit, and each of those has its own house style for
/// yes. Anything else — including the empty string, which is what an unset
/// variable often expands to — is off, so a typo fails safe by leaving the
/// cookie usable over the HTTP the server actually speaks.
fn truthy(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "yes" | "on"
    )
}

impl Config {
    /// The headless server's configuration: the environment, then the working
    /// directory.
    pub fn from_env() -> Result<Self, ConfigError> {
        Self::resolve(Fallbacks::default())
    }

    /// The environment first, then `fallbacks` for whatever it did not say.
    ///
    /// The environment always wins. Somebody who sets `RHIZOLOG_ROOT` before
    /// launching the app means it, and a remembered choice quietly overriding
    /// them would be the wrong way round.
    pub fn resolve(fallbacks: Fallbacks) -> Result<Self, ConfigError> {
        Self::layer(Overrides::from_env(), fallbacks)
    }

    /// The precedence itself, with the environment already read.
    ///
    /// Separate so it can be tested: `std::env::set_var` is unsafe in edition
    /// 2024 and racy between parallel tests, which is a poor foundation for the
    /// one piece of logic here that anybody would get wrong.
    fn layer(overrides: Overrides, fallbacks: Fallbacks) -> Result<Self, ConfigError> {
        let root = overrides
            .root
            .or(fallbacks.root)
            .ok_or(ConfigError::NoWikiRoot)?;

        let database = overrides
            .database
            .unwrap_or_else(|| root.join(INTERNAL_DIR).join("index.db"));

        let listen = match overrides.address {
            Some(value) => {
                let address = value.parse().map_err(|source| ConfigError::Address {
                    value: value.clone(),
                    source,
                })?;
                Listen::Exactly(address)
            }
            None => fallbacks.listen,
        };

        let assets = overrides.assets.unwrap_or(fallbacks.assets);

        Ok(Self {
            root,
            database,
            listen,
            assets,
            secure_cookies: overrides.secure_cookies.unwrap_or(false),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fallbacks() -> Fallbacks {
        Fallbacks {
            root: Some(PathBuf::from("/fallback/wiki")),
            assets: PathBuf::from("/fallback/dist"),
            listen: Listen::Preferably(DEFAULT_ADDRESS),
        }
    }

    #[test]
    fn fallbacks_are_used_when_the_environment_says_nothing() {
        let config = Config::layer(Overrides::default(), fallbacks()).expect("config");

        assert_eq!(config.root, PathBuf::from("/fallback/wiki"));
        assert_eq!(config.assets, PathBuf::from("/fallback/dist"));
        assert_eq!(config.listen, Listen::Preferably(DEFAULT_ADDRESS));
    }

    /// The point of the layering: a variable set before launch means it, and a
    /// remembered choice must not quietly win over somebody who typed one.
    #[test]
    fn the_environment_wins_over_every_fallback() {
        let overrides = Overrides {
            root: Some(PathBuf::from("/env/wiki")),
            database: Some(PathBuf::from("/env/index.db")),
            address: Some("127.0.0.1:9999".to_owned()),
            assets: Some(PathBuf::from("/env/dist")),
            secure_cookies: Some(true),
        };

        let config = Config::layer(overrides, fallbacks()).expect("config");

        assert_eq!(config.root, PathBuf::from("/env/wiki"));
        assert_eq!(config.database, PathBuf::from("/env/index.db"));
        assert_eq!(config.assets, PathBuf::from("/env/dist"));
        assert_eq!(
            config.listen,
            Listen::Exactly("127.0.0.1:9999".parse().unwrap())
        );
        assert!(config.secure_cookies);
    }

    /// A cookie marked `Secure` is discarded by a browser that received it over
    /// HTTP, which is what the server speaks. Defaulting this on would make
    /// every sign-in return 200 and leave the user signed out.
    #[test]
    fn cookies_are_not_marked_secure_unless_asked() {
        let config = Config::layer(Overrides::default(), fallbacks()).expect("config");
        assert!(!config.secure_cookies);
    }

    /// Set in a shell, a Dockerfile or a unit file, each with its own house
    /// style for yes — and anything unrecognised has to fail safe.
    #[test]
    fn a_boolean_variable_accepts_the_usual_spellings() {
        for value in ["1", "true", "TRUE", "yes", "on", " true "] {
            assert!(truthy(value), "{value:?} should be on");
        }
        for value in ["", "0", "false", "no", "off", "maybe"] {
            assert!(!truthy(value), "{value:?} should be off");
        }
    }

    /// An address someone wrote down is `Exactly`; a fallback is a preference.
    /// The difference is the whole reason `Listen` is not a `SocketAddr`.
    #[test]
    fn an_address_from_the_environment_is_binding_and_a_fallback_is_not() {
        let from_env = Config::layer(
            Overrides {
                address: Some("127.0.0.1:8080".to_owned()),
                ..Overrides::default()
            },
            fallbacks(),
        )
        .expect("config");
        assert!(matches!(from_env.listen, Listen::Exactly(_)));

        let from_fallback = Config::layer(Overrides::default(), fallbacks()).expect("config");
        assert!(matches!(from_fallback.listen, Listen::Preferably(_)));
    }

    /// The database follows whichever root won, rather than the one that lost.
    #[test]
    fn the_database_defaults_under_the_root_that_was_chosen() {
        let overrides = Overrides {
            root: Some(PathBuf::from("/env/wiki")),
            ..Overrides::default()
        };

        let config = Config::layer(overrides, fallbacks()).expect("config");

        assert_eq!(
            config.database,
            PathBuf::from("/env/wiki")
                .join(INTERNAL_DIR)
                .join("index.db")
        );
    }

    /// A caller with nothing to offer and an environment that says nothing
    /// either has to fail loudly. `Store::open` creates its root, so the
    /// alternative is an empty wiki materialised wherever the process happened
    /// to be standing.
    #[test]
    fn no_root_anywhere_is_an_error_rather_than_a_guess() {
        let fallbacks = Fallbacks {
            root: None,
            ..fallbacks()
        };

        assert!(matches!(
            Config::layer(Overrides::default(), fallbacks),
            Err(ConfigError::NoWikiRoot)
        ));
    }

    #[test]
    fn a_root_from_the_environment_covers_a_caller_that_has_none() {
        let overrides = Overrides {
            root: Some(PathBuf::from("/env/wiki")),
            ..Overrides::default()
        };
        let fallbacks = Fallbacks {
            root: None,
            ..fallbacks()
        };

        let config = Config::layer(overrides, fallbacks).expect("config");
        assert_eq!(config.root, PathBuf::from("/env/wiki"));
    }
}
