//! The published endpoint: where a running server can be reached.
//!
//! A server that may end up on a different port from the one it asked for has
//! to say where it landed, or nothing can find it. `.rhizolog/server.json` is
//! that answer, and it sits beside the wiki because the wiki directory is the
//! handle a caller already has — an agent working in a checkout knows the
//! directory, and should not also have to be told a port.
//!
//! ## Written last, so its presence means something
//!
//! [`crate::server::start`] publishes it only once the server is ready — bound,
//! reconciled, and watching. A reader that finds one therefore does not have to
//! poll for a usable index, and no "starting up" state has to be invented to
//! describe the gap. A clean shutdown withdraws it again.
//!
//! ## A hint, not proof
//!
//! A process killed hard leaves the file behind, and process ids get reused, so
//! the file existing is not evidence that anything is listening. A reader
//! confirms by asking `GET /api/health` and checking the `wiki_root` it reports
//! against the wiki it meant. That handshake costs one request and cannot be
//! fooled the way a liveness check on a recorded pid can — which is why the pid
//! here is for a human reading the file, not for a program branching on it.
//!
//! ## A third kind of thing under `.rhizolog/`
//!
//! `index.db` is derived and rebuilds from the wiki; `times/` is authored and
//! has no other copy. This is neither. It is **volatile**: meaningless the
//! moment the process that wrote it stops, never worth preserving, backing up
//! or restoring. It is gitignored by name beside the database, for the reason
//! [`crate::store::INTERNAL_DIR`] cannot be ignored wholesale.

use std::io;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

use crate::store::{INTERNAL_DIR, display_path, write_atomically};

/// The file's name inside `.rhizolog/`.
pub const ENDPOINT_FILE: &str = "server.json";

/// What a running server publishes about itself.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Endpoint {
    /// Where to reach it, ready to have a path appended.
    pub url: String,
    /// The wiki being served. A file describing a different wiki is a file that
    /// was left somewhere it does not belong.
    pub wiki_root: String,
    /// The process that wrote this. For a person reading the file; see the
    /// module docs for why nothing should branch on it.
    pub pid: u32,
    pub version: String,
    pub started: DateTime<Utc>,
}

impl Endpoint {
    pub fn new(address: SocketAddr, wiki_root: &Path) -> Self {
        Self {
            url: format!("http://{address}"),
            wiki_root: display_path(wiki_root),
            pid: std::process::id(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            started: Utc::now(),
        }
    }
}

/// Where the endpoint file lives for the wiki at `root`.
pub fn path(root: &Path) -> PathBuf {
    root.join(INTERNAL_DIR).join(ENDPOINT_FILE)
}

/// Publish `endpoint` for the wiki at `root`.
pub async fn publish(root: &Path, endpoint: &Endpoint) -> io::Result<()> {
    let path = path(root);
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    let json = serde_json::to_vec_pretty(endpoint).map_err(io::Error::from)?;
    // Atomically: a reader can arrive at any moment, and half a JSON document
    // is a parse error rather than a smaller answer.
    write_atomically(&path, &json).await
}

/// Remove the endpoint file for the wiki at `root`.
pub async fn withdraw(root: &Path) -> io::Result<()> {
    match tokio::fs::remove_file(path(root)).await {
        Ok(()) => Ok(()),
        // Already gone is the state being asked for, not a failure to reach it.
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

/// Read the endpoint published for the wiki at `root`, if there is one.
///
/// Missing, unreadable and malformed all come back as `None`. This is a hint,
/// and a hint nobody can read is no hint — there is nothing a caller could
/// usefully do differently for a file that is corrupt rather than absent, and
/// in both cases the next step is the same: assume nothing is running.
pub async fn read(root: &Path) -> Option<Endpoint> {
    let bytes = tokio::fs::read(path(root)).await.ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// How long to wait for a published endpoint to answer.
///
/// Generous for a loopback round trip on a busy machine, short enough that a
/// file left behind by last week's crash does not make a launch feel broken.
const CONFIRM_TIMEOUT: Duration = Duration::from_secs(2);

/// The endpoint published for `root`, but only if a server is really there.
///
/// This is the discovery protocol in one function: read the hint, then confirm
/// it. Anything that wants to reach a running Rhizolog should go through here
/// rather than trusting the file, and anything that wants to know whether a
/// wiki is *already being served* — a second copy of the desktop app, say —
/// is asking the same question.
///
/// Two ways the hint lies, and both are checked by the one request:
///
/// - **The server is gone.** A hard kill leaves the file, and nothing answers.
/// - **Something else has the port.** Process ids and ports both get reused, so
///   an answer is not enough; it has to be an answer about *this* wiki. The
///   comparison is against the root actually asked about rather than the one
///   the file names, because a copied wiki directory brings its `server.json`
///   with it and that file describes somebody else's live server.
pub async fn live(root: &Path) -> Option<Endpoint> {
    let endpoint = read(root).await?;

    // A root that cannot be canonicalised does not exist, and nothing can be
    // serving a wiki that is not there.
    let canonical = tokio::fs::canonicalize(root).await.ok()?;
    let expected = display_path(&canonical);

    let address: SocketAddr = endpoint.url.strip_prefix("http://")?.parse().ok()?;
    let reported = tokio::time::timeout(CONFIRM_TIMEOUT, serving(address))
        .await
        .ok()??;

    (reported == expected).then_some(endpoint)
}

/// The wiki a server at `address` says it is serving, if one answers at all.
async fn serving(address: SocketAddr) -> Option<String> {
    let mut stream = TcpStream::connect(address).await.ok()?;
    let request =
        format!("GET /api/health HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.ok()?;

    // `Connection: close` is what makes reading to the end terminate, and it
    // saves having to parse a content length to find out where to stop.
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.ok()?;

    let body = body_of(&response)?;
    let health: serde_json::Value = serde_json::from_slice(body).ok()?;
    health.get("wiki_root")?.as_str().map(str::to_owned)
}

fn body_of(response: &[u8]) -> Option<&[u8]> {
    let blank_line = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")?;
    Some(&response[blank_line + 4..])
}

#[cfg(test)]
mod tests {
    use std::net::{IpAddr, Ipv4Addr};

    use tempfile::TempDir;

    use super::*;

    fn endpoint(root: &Path) -> Endpoint {
        Endpoint::new(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 3000), root)
    }

    #[tokio::test]
    async fn an_endpoint_round_trips() {
        let directory = TempDir::new().expect("temp dir");
        let root = directory.path();
        let published = endpoint(root);

        publish(root, &published).await.expect("publish");

        assert_eq!(read(root).await.as_ref(), Some(&published));
        assert_eq!(published.url, "http://127.0.0.1:3000");
        assert_eq!(published.pid, std::process::id());
    }

    #[tokio::test]
    async fn withdrawing_leaves_nothing_and_can_be_repeated() {
        let directory = TempDir::new().expect("temp dir");
        let root = directory.path();

        publish(root, &endpoint(root)).await.expect("publish");
        withdraw(root).await.expect("withdraw");

        assert_eq!(read(root).await, None);
        withdraw(root)
            .await
            .expect("withdrawing an absent endpoint should be fine");
    }

    /// A truncated or hand-mangled file is a hint nobody can read, which is the
    /// same situation as no hint at all.
    #[tokio::test]
    async fn a_malformed_endpoint_reads_as_absent() {
        let directory = TempDir::new().expect("temp dir");
        let root = directory.path();

        tokio::fs::create_dir_all(root.join(INTERNAL_DIR))
            .await
            .expect("internal directory");
        tokio::fs::write(path(root), "{\"url\": \"http://127.0.0.1:")
            .await
            .expect("write half a document");

        assert_eq!(read(root).await, None);
    }

    #[tokio::test]
    async fn reading_a_wiki_with_no_server_is_none_rather_than_an_error() {
        let directory = TempDir::new().expect("temp dir");
        assert_eq!(read(directory.path()).await, None);
    }
}
