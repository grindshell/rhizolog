//! Rhizolog — a wiki backend over a directory of markdown files.
//!
//! Markdown files on disk are the source of truth; the SQLite index is derived
//! and can be rebuilt at any time. See `knowledge-base/architecture.md` for the
//! reasoning, and `knowledge-base/api-design.md` for the endpoint surface.

pub mod api;
pub mod assets;
pub mod auth;
pub mod compile;
pub mod config;
pub mod endpoint;
pub mod error;
pub mod frontmatter;
pub mod ideas;
pub mod index;
pub mod markdown;
pub mod page;
pub mod prose;
pub mod server;
pub mod slug;
pub mod store;
pub mod times;
pub mod users;
pub mod watcher;

pub use api::usage::UsageTally;
pub use api::{AppState, router};
pub use assets::Assets;
pub use auth::Viewer;
pub use compile::{CompileError, Compiled, Section};
pub use config::{Config, Fallbacks, Listen};
pub use endpoint::Endpoint;
pub use error::{AppError, AppResult};
pub use ideas::{
    Capture, CaptureId, Event, EventId, EventKind, Idea, IdeaId, IdeaService, IdeaServiceError,
    IdeaStore, IdeaStoreError, Owner, Subject,
};
pub use index::{Index, IndexError, SyncReport};
pub use page::{Frontmatter, Page, PageError};
pub use prose::{Analysis, Finding, ProseError, Rule, Ruleset, Severity};
pub use server::Server;
pub use slug::{Slug, SlugError};
pub use store::{Store, StoreError};
pub use times::{TimeEntry, TimeId, TimeStore, TimeStoreError};
pub use users::{Role, User, UserStore, UserStoreError, Username};
