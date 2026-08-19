//! The server's lifecycle: starting, serving, and stopping cleanly.
//!
//! `server::start` and `Server::shutdown` are the whole surface a shell around
//! the server gets — see `knowledge-base/desktop-app.md` — so the two things
//! they promise are worth asserting rather than assuming: that the address is
//! real and the index complete by the time `start` returns, and that nothing is
//! left listening or unflushed after `shutdown`.

use std::net::SocketAddr;

use rhizolog::{Config, Endpoint, Index, Listen, endpoint, server};
use tempfile::TempDir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

fn config(directory: &TempDir) -> Config {
    listening(
        directory,
        Listen::Exactly(SocketAddr::from(([127, 0, 0, 1], 0))),
    )
}

fn listening(directory: &TempDir, listen: Listen) -> Config {
    Config {
        root: directory.path().to_path_buf(),
        database: directory.path().join(".rhizolog").join("index.db"),
        // Port 0 unless a test says otherwise, so these never fight each other
        // — or the developer's own server — over 3000.
        listen,
        // Nothing is built in a temp directory, which is a normal state rather
        // than an error.
        assets: directory.path().join("dist"),
        // These are HTTP, so a `Secure` cookie would be one the client throws
        // away — which is also the default, and the reason it is.
        secure_cookies: false,
    }
}

/// Hold a port open for as long as the returned listener lives.
async fn occupied() -> (TcpListener, SocketAddr) {
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .expect("occupy a port");
    let address = listener.local_addr().expect("its address");
    (listener, address)
}

/// One HTTP/1.1 request, by hand.
///
/// The crate has no HTTP client and does not need one for this. Going over a
/// real socket is the whole point — these tests are about the listener, which
/// calling the `Router` in process would never touch. `Connection: close` is
/// what lets the response be read to EOF without parsing a content length.
async fn get(address: SocketAddr, path: &str) -> String {
    let mut stream = TcpStream::connect(address).await.expect("connect");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n");
    stream.write_all(request.as_bytes()).await.expect("send");

    let mut response = String::new();
    stream.read_to_string(&mut response).await.expect("read");
    response
}

#[tokio::test]
async fn starts_ready_and_stops_clean() {
    let directory = TempDir::new().expect("temp dir");
    let server = server::start(&config(&directory)).await.expect("start");
    let address = server.address();

    assert_ne!(
        address.port(),
        0,
        "start returned before the bind had resolved a port"
    );

    let response = get(address, "/api/health").await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(response.contains("\"status\":\"ok\""), "{response}");

    server.shutdown().await.expect("shutdown");

    assert!(
        TcpStream::connect(address).await.is_err(),
        "the listener outlived the shutdown"
    );
}

/// `start` returning means ready, not starting. Anything that publishes the
/// address the moment it has one — a desktop window, eventually a file agents
/// read — hands out a URL that a request can arrive at immediately, and that
/// request must not see a half-built index.
#[tokio::test]
async fn the_wiki_is_reconciled_before_the_address_is_handed_back() {
    let directory = TempDir::new().expect("temp dir");
    tokio::fs::write(
        directory.path().join("rhizome.md"),
        "---\ntitle: Rhizome\n---\n\nKnowledge branches off chaotically.\n",
    )
    .await
    .expect("write page");

    let server = server::start(&config(&directory)).await.expect("start");
    let response = get(server.address(), "/api/health").await;

    assert!(
        response.contains("\"pages\":1"),
        "the page written before startup was not indexed yet: {response}"
    );

    server.shutdown().await.expect("shutdown");
}

/// API usage is tallied in memory and flushed every sixty seconds, so a
/// shutdown that forgets the final flush loses up to a minute of counts and
/// loses them silently — the only symptom is a number in `/api/stats` that is
/// quietly too low. A window closing is now one of the ways to exit.
#[tokio::test]
async fn shutting_down_persists_the_usage_tally() {
    let directory = TempDir::new().expect("temp dir");
    let config = config(&directory);
    let server = server::start(&config).await.expect("start");
    let address = server.address();

    get(address, "/api/health").await;
    get(address, "/api/health").await;
    get(address, "/api/pages").await;

    server.shutdown().await.expect("shutdown");

    // Reopened from the file rather than read through the handle the server
    // was holding, so this is an assertion about what reached the disk.
    let index = Index::open(Some(&config.database))
        .await
        .expect("reopen the index");
    let counts: Vec<(String, String, u64)> = index
        .usage()
        .await
        .expect("usage")
        .into_iter()
        .map(|usage| (usage.route, usage.method, usage.count))
        .collect();

    assert!(
        counts.contains(&("/api/health".to_owned(), "GET".to_owned(), 2)),
        "{counts:?}"
    );
    assert!(
        counts.contains(&("/api/pages".to_owned(), "GET".to_owned(), 1)),
        "{counts:?}"
    );
}

/// The endpoint file is how anything finds a server that may not be on the port
/// it asked for. It has to appear only when there is something ready to find,
/// and it has to go away again — a file pointing at a server that has stopped
/// is worse than no file, because it reads as an answer.
#[tokio::test]
async fn the_endpoint_is_published_while_the_server_is_up_and_withdrawn_after() {
    let directory = TempDir::new().expect("temp dir");
    let server = server::start(&config(&directory)).await.expect("start");
    let address = server.address();

    let published = endpoint::read(directory.path())
        .await
        .expect("no endpoint was published");

    assert_eq!(published.url, format!("http://{address}"));
    assert_eq!(published.pid, std::process::id());
    assert_eq!(published.version, env!("CARGO_PKG_VERSION"));

    // The url is the point of the file, so follow it rather than trusting it.
    let followed: SocketAddr = published
        .url
        .trim_start_matches("http://")
        .parse()
        .expect("the published url should parse as an address");
    let response = get(followed, "/api/health").await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    assert!(
        response.contains(&published.wiki_root.replace('\\', "\\\\")),
        "the endpoint and /api/health disagree about which wiki this is: \
         {} vs {response}",
        published.wiki_root
    );

    server.shutdown().await.expect("shutdown");

    assert_eq!(
        endpoint::read(directory.path()).await,
        None,
        "the endpoint outlived the server it described"
    );
}

/// `endpoint::live` is the whole discovery protocol, and the question a second
/// copy of the desktop app asks before it starts anything: is this wiki already
/// being served?
#[tokio::test]
async fn a_served_wiki_is_confirmed_live_and_stops_being_so_when_it_stops() {
    let directory = TempDir::new().expect("temp dir");
    let server = server::start(&config(&directory)).await.expect("start");

    let live = endpoint::live(directory.path())
        .await
        .expect("a running server was not confirmed");
    assert_eq!(live.url, format!("http://{}", server.address()));

    server.shutdown().await.expect("shutdown");

    assert_eq!(
        endpoint::live(directory.path()).await,
        None,
        "a stopped server is still being reported as live"
    );
}

/// The reason the file alone is not an answer. A hard kill leaves one behind
/// naming a port nothing is listening on, and treating that as proof would mean
/// refusing to open a wiki because of a server that died last week.
#[tokio::test]
async fn an_endpoint_nothing_answers_at_is_not_live() {
    let directory = TempDir::new().expect("temp dir");
    // Bound and dropped: as close to a certainly-free port as this can get.
    let (listener, address) = occupied().await;
    drop(listener);

    endpoint::publish(directory.path(), &Endpoint::new(address, directory.path()))
        .await
        .expect("publish by hand");

    assert!(endpoint::read(directory.path()).await.is_some(), "setup");
    assert_eq!(endpoint::live(directory.path()).await, None);
}

/// Ports get reused, and a wiki directory can be copied with its `server.json`
/// inside it. An answer is therefore not enough on its own — it has to be an
/// answer about the wiki that was asked about.
#[tokio::test]
async fn an_endpoint_answering_for_a_different_wiki_is_not_live() {
    let served = TempDir::new().expect("temp dir");
    let copy = TempDir::new().expect("temp dir");

    let server = server::start(&config(&served)).await.expect("start");

    // What copying a wiki directory does: the file arrives describing a server
    // that is genuinely running, for somebody else's wiki.
    let borrowed = endpoint::read(served.path()).await.expect("published");
    endpoint::publish(copy.path(), &borrowed)
        .await
        .expect("publish the copy");

    assert_eq!(
        endpoint::live(copy.path()).await,
        None,
        "a live server for another wiki was accepted as this one's"
    );
    assert!(
        endpoint::live(served.path()).await.is_some(),
        "the wiki actually being served stopped being recognised"
    );

    server.shutdown().await.expect("shutdown");
}

/// A second copy finding 3000 taken is an ordinary Tuesday, and refusing to
/// start would be a poor answer to it. The published endpoint is what makes
/// moving survivable.
#[tokio::test]
async fn a_preferred_address_that_is_taken_gives_way() {
    let (held, taken) = occupied().await;
    let directory = TempDir::new().expect("temp dir");

    let server = server::start(&listening(&directory, Listen::Preferably(taken)))
        .await
        .expect("start should fall back rather than fail");

    let address = server.address();
    assert_ne!(address.port(), taken.port(), "it bound the occupied port");
    assert_ne!(address.port(), 0);
    assert_eq!(address.ip(), taken.ip(), "the fallback changed host");

    let published = endpoint::read(directory.path()).await.expect("endpoint");
    assert_eq!(
        published.url,
        format!("http://{address}"),
        "the endpoint records where it wanted to be, not where it is"
    );

    let response = get(address, "/api/health").await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");

    server.shutdown().await.expect("shutdown");
    drop(held);
}

/// An address somebody wrote down is a requirement, not a preference. Serving
/// somewhere else would leave whatever they wrote it down in pointing at
/// nothing, which is a worse failure than not starting.
#[tokio::test]
async fn an_exact_address_that_is_taken_is_an_error() {
    let (held, taken) = occupied().await;
    let directory = TempDir::new().expect("temp dir");

    let result = server::start(&listening(&directory, Listen::Exactly(taken))).await;

    assert!(result.is_err(), "it started on an address already in use");
    assert_eq!(
        endpoint::read(directory.path()).await,
        None,
        "a server that never started published an endpoint anyway"
    );

    drop(held);
}

/// Two servers over one wiki is a thing the desktop app has to prevent rather
/// than a thing this asserts is fine — but starting one, stopping it, and
/// starting another is the ordinary restart, and it only works if shutdown
/// really let go of the index and the port.
#[tokio::test]
async fn a_wiki_can_be_served_again_after_a_shutdown() {
    let directory = TempDir::new().expect("temp dir");
    let config = config(&directory);

    let first = server::start(&config).await.expect("first start");
    get(first.address(), "/api/health").await;
    first.shutdown().await.expect("first shutdown");

    let second = server::start(&config).await.expect("second start");
    let response = get(second.address(), "/api/health").await;
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    second.shutdown().await.expect("second shutdown");
}
