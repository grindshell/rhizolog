//! Where the built dashboard is served from.
//!
//! Two answers, and the difference is where the bytes live rather than what
//! they are. A server run from a checkout points at `frontend/dist` on disk; a
//! build that has to travel on its own carries the same files inside the
//! executable, because a portable binary cannot point at a directory that
//! travels separately from it.
//!
//! ## Why the server serves them at all
//!
//! The obvious alternative for the desktop app is to let its shell serve the
//! SPA over a native protocol and leave the HTTP server with `/api`. That is
//! where local and remote quietly diverge. A remote instance serves its UI over
//! HTTP, so if the local one did not, then the SPA fallback, the `/api`
//! catch-all that keeps a mistyped endpoint from being answered with HTML, and
//! every same-origin assumption in the client would each be exercised in only
//! one of the two configurations. One code path, one set of tests, one
//! application. See `knowledge-base/desktop-app.md`.
//!
//! ## The two are meant to be indistinguishable
//!
//! [`Assets::Embedded`] follows the same rule [`Assets::Dir`] does: a request
//! that names a real file gets that file, and anything else gets `index.html`,
//! because a client route has no file behind it and a hard refresh must still
//! work. `tests/frontend.rs` asserts that of both.

use std::path::{Path, PathBuf};

use axum::http::{StatusCode, Uri, header};
use axum::response::{IntoResponse, Response};

/// The app shell, and the answer to anything that is not a file.
const INDEX: &str = "index.html";

/// Where the built dashboard is, if it is anywhere.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Assets {
    /// A directory on disk. What a checkout and `RHIZOLOG_ASSETS` produce.
    Dir(PathBuf),
    /// Compiled into this binary, under the `embed-assets` feature.
    Embedded,
    /// Nothing has been built. A normal state, not an error: during frontend
    /// development `pnpm dev` serves the UI itself and proxies the API here.
    None,
}

/// Decide what to serve, preferring a directory that actually exists.
///
/// A directory wins over the embedded copy on purpose. Somebody who has pointed
/// `RHIZOLOG_ASSETS` at a fresh build wants that build, not the one compiled in
/// weeks ago — and the log line says which was chosen, so the answer is never a
/// guess.
pub async fn resolve(directory: &Path) -> Assets {
    if matches!(tokio::fs::try_exists(directory).await, Ok(true)) {
        tracing::info!(path = %directory.display(), "serving the built frontend");
        return Assets::Dir(directory.to_path_buf());
    }

    let embedded = embedded_paths();
    if !embedded.is_empty() {
        tracing::info!(
            files = embedded.len(),
            "serving the frontend embedded in this binary"
        );
        return Assets::Embedded;
    }

    tracing::info!(
        path = %directory.display(),
        "no frontend build found; serving the API only"
    );
    Assets::None
}

/// Serve a request from the embedded copy.
///
/// Present whether or not anything was embedded, so the router does not have to
/// be written twice. With the feature off there is nothing to find, and the
/// answer is the same one a missing build gets.
pub(crate) async fn serve(uri: Uri) -> Response {
    let requested = uri.path().trim_start_matches('/');
    let requested = if requested.is_empty() {
        INDEX
    } else {
        requested
    };

    // A real file wins; anything else is a client route and gets the shell.
    // The same rule `ServeDir::fallback(ServeFile)` follows for a directory.
    match get(requested).or_else(|| get(INDEX)) {
        Some((mimetype, data)) => ([(header::CONTENT_TYPE, mimetype)], data).into_response(),
        None => (StatusCode::NOT_FOUND, crate::api::NO_FRONTEND).into_response(),
    }
}

/// The paths of every embedded file.
///
/// Empty when the feature is off, and empty when it is on but nothing was
/// built — the derive reads the directory at compile time, so an embed with no
/// `index.html` in it is a build that never ran. Either way there is nothing to
/// serve, and [`resolve`] treats it as such rather than serving a shell that
/// does not exist.
pub fn embedded_paths() -> Vec<String> {
    #[cfg(feature = "embed-assets")]
    {
        embedded::Files::iter()
            .map(|path| path.to_string())
            .collect()
    }
    #[cfg(not(feature = "embed-assets"))]
    {
        Vec::new()
    }
}

/// One embedded file, as its content type and its bytes.
fn get(path: &str) -> Option<(String, std::borrow::Cow<'static, [u8]>)> {
    #[cfg(feature = "embed-assets")]
    {
        let file = embedded::Files::get(path)?;
        Some((file.metadata.mimetype().to_owned(), file.data))
    }
    #[cfg(not(feature = "embed-assets"))]
    {
        let _ = path;
        None
    }
}

#[cfg(feature = "embed-assets")]
mod embedded {
    /// The built dashboard, read at compile time.
    ///
    /// The path is relative to this crate's manifest, so it is the same
    /// `frontend/dist` the directory case points at by default. Building with
    /// this feature and no `pnpm build` behind it is a mistake the compiler
    /// makes for you.
    #[derive(rust_embed::RustEmbed)]
    #[folder = "../frontend/dist"]
    pub struct Files;
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[tokio::test]
    async fn a_directory_that_exists_is_served_from_disk() {
        let directory = TempDir::new().expect("temp dir");
        assert_eq!(
            resolve(directory.path()).await,
            Assets::Dir(directory.path().to_path_buf())
        );
    }

    /// Without the feature there is nothing compiled in, so a missing directory
    /// leaves nothing to serve — and that has to stay a normal state rather
    /// than an error, because it is what frontend development looks like.
    #[cfg(not(feature = "embed-assets"))]
    #[tokio::test]
    async fn a_missing_directory_with_nothing_embedded_is_none() {
        let directory = TempDir::new().expect("temp dir");
        let missing = directory.path().join("never-built");

        assert_eq!(resolve(&missing).await, Assets::None);
        assert!(embedded_paths().is_empty());
    }

    /// With the feature on, the compiled-in copy is the safety net under a
    /// directory that is not there — which is the whole point of it, since a
    /// portable binary is never run beside a checkout.
    #[cfg(feature = "embed-assets")]
    #[tokio::test]
    async fn a_missing_directory_falls_back_to_the_embedded_copy() {
        let directory = TempDir::new().expect("temp dir");
        let missing = directory.path().join("never-built");

        assert_eq!(resolve(&missing).await, Assets::Embedded);
        assert!(
            embedded_paths().iter().any(|path| path == INDEX),
            "nothing usable was embedded: {:?}",
            embedded_paths()
        );
    }
}
