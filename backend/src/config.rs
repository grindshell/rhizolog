//! Environment-driven configuration.

use std::env;
use std::net::SocketAddr;
use std::path::PathBuf;

use thiserror::Error;

use crate::store::INTERNAL_DIR;

pub const ENV_ROOT: &str = "RHIZOLOG_ROOT";
pub const ENV_DATABASE: &str = "RHIZOLOG_DB";
pub const ENV_ADDRESS: &str = "RHIZOLOG_ADDR";
pub const ENV_ASSETS: &str = "RHIZOLOG_ASSETS";
pub const ENV_LOG: &str = "RHIZOLOG_LOG";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    /// The wiki directory. Markdown files here are the source of truth.
    pub root: PathBuf,
    /// The derived SQLite index. Safe to delete; it rebuilds on startup.
    pub database: PathBuf,
    pub address: SocketAddr,
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

        let address = match env::var(ENV_ADDRESS) {
            Ok(value) => value.parse().map_err(|source| ConfigError::Address {
                value: value.clone(),
                source,
            })?,
            // Loopback by default, deliberately: Rhizolog is single-user, has
            // no authentication, and its API writes files.
            Err(_) => SocketAddr::from(([127, 0, 0, 1], 3000)),
        };

        let assets = env::var_os(ENV_ASSETS)
            .map(PathBuf::from)
            // Relative to the repository root, which is where `cargo run` is
            // usually invoked from via `backend/`.
            .unwrap_or_else(|| PathBuf::from("../frontend/dist"));

        Ok(Self {
            root,
            database,
            address,
            assets,
        })
    }
}
