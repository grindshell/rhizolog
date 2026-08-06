//! Rhizolog — a wiki backend over a directory of markdown files.
//!
//! Markdown files on disk are the source of truth; the SQLite index is derived
//! and can be rebuilt at any time. See `knowledge-base/architecture.md` for the
//! reasoning, and `knowledge-base/api-design.md` for the endpoint surface.

pub mod api;
pub mod config;
pub mod error;
pub mod frontmatter;
pub mod index;
pub mod markdown;
pub mod page;
pub mod slug;
pub mod store;
pub mod times;
pub mod watcher;

pub use api::usage::UsageTally;
pub use api::{AppState, router};
pub use config::Config;
pub use error::{AppError, AppResult};
pub use index::{Index, IndexError, SyncReport};
pub use page::{Frontmatter, Page, PageError};
pub use slug::{Slug, SlugError};
pub use store::{Store, StoreError};
pub use times::{TimeEntry, TimeId, TimeStore, TimeStoreError};
