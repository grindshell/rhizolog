//! Write `frontend/openapi.json` without running a server.
//!
//! ```text
//! cargo run --example dump-openapi
//! ```
//!
//! The obvious way to refresh the spec is to `curl` it off a running backend,
//! and on Windows that is a trap with two jaws. PowerShell 5.1's `curl` is
//! `Invoke-WebRequest`, which decodes a body as Latin-1 when the `Content-Type`
//! carries no charset — and `application/json` does not — so every em-dash in
//! the document turns from `E2 80 94` into `C3 A2 C2 80 C2 94`. Redirecting
//! with `>` then re-encodes it again and adds a BOM. The result is still valid
//! JSON on one line, so the diff looks like an ordinary regeneration and
//! nothing catches it.
//!
//! Writing the bytes from here sidesteps both, needs no server, and means the
//! spec can be regenerated in a checkout that has never been run.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    // `api::openapi()`, not `ApiDoc::openapi()`. The derive only produces the
    // `info` and `tags` skeleton; every path comes from the routes, and this
    // is the same function the server publishes from.
    //
    // Compact rather than pretty, matching what the server serves: the file is
    // an input to `pnpm gen:api`, not something anyone reads, and keeping the
    // two spellings identical means a regenerated file can be compared against
    // a fetched one byte for byte.
    let json = rhizolog::api::openapi()
        .to_json()
        .expect("the OpenAPI document serialises");

    // Relative to the crate root, which is where `cargo run` starts.
    let target = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("../frontend/openapi.json"));

    // Bytes, UTF-8, no BOM, exactly as produced.
    std::fs::write(&target, json.as_bytes())?;

    println!("wrote {} ({} bytes)", target.display(), json.len());
    Ok(())
}
