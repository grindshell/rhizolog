//! Rhizowiki — a wiki backend over a directory of markdown files.
//!
//! Markdown files on disk are the source of truth; the SQLite index is derived
//! and can be rebuilt at any time. See `knowledge-base/architecture.md` for the
//! reasoning, and `knowledge-base/api-design.md` for the endpoint surface.

pub mod api;
pub mod config;
pub mod error;
pub mod index;
pub mod markdown;
pub mod page;
pub mod slug;
pub mod store;

pub use api::{AppState, router};
pub use config::Config;
pub use error::{AppError, AppResult};
pub use index::{Index, IndexError, SyncReport};
pub use page::{Frontmatter, Page, PageError};
pub use slug::{Slug, SlugError};
pub use store::{Store, StoreError};
