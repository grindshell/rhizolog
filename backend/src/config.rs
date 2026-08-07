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

/// Loopback, deliberately: Rhizolog is single-user, has no authentication, and
/// its API writes files.
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
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("{ENV_ADDRESS} is not a valid socket address: {value:?}")]
    Address {
        value: String,
        #[source]
        source: std::net::AddrParseError,
    },
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        let root = env::var_os(ENV_ROOT)
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("./wiki"));

        let database = env::var_os(ENV_DATABASE)
            .map(PathBuf::from)
            .unwrap_or_else(|| root.join(INTERNAL_DIR).join("index.db"));

        let listen = match env::var(ENV_ADDRESS) {
            Ok(value) => {
                let address = value.parse().map_err(|source| ConfigError::Address {
                    value: value.clone(),
                    source,
                })?;
                Listen::Exactly(address)
            }
            Err(_) => Listen::Preferably(DEFAULT_ADDRESS),
        };

        let assets = env::var_os(ENV_ASSETS)
            .map(PathBuf::from)
            // Relative to the repository root, which is where `cargo run` is
            // usually invoked from via `backend/`.
            .unwrap_or_else(|| PathBuf::from("../frontend/dist"));

        Ok(Self {
            root,
            database,
            listen,
            assets,
        })
    }
}
