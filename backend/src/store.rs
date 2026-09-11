//! The wiki directory.
//!
//! Every filesystem path in Rhizolog is built here, from a validated
//! [`Slug`], under a canonicalised root. [`Slug::parse`] already rejects
//! traversal, so the remaining escape route is a symlink pointing out of the
//! wiki — [`Store::resolve`] closes that one.
//!
//! Writes go to a temporary file and are then renamed into place. That keeps a
//! reader (or the file watcher) from ever seeing a half-written page, and makes
//! a save one filesystem event instead of several.

use std::io;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use thiserror::Error;
use walkdir::WalkDir;

use crate::page::{Frontmatter, Page, PageError};
use crate::slug::Slug;

/// Directory holding the derived index, and anything else Rhizolog needs to
/// keep inside the wiki without treating it as content.
pub const INTERNAL_DIR: &str = ".rhizolog";

#[derive(Debug, Error)]
pub enum StoreError {
    #[error("no page at {slug}")]
    NotFound { slug: Slug },

    #[error("a page already exists at {slug}")]
    AlreadyExists { slug: Slug },

    #[error("{slug} resolves outside the wiki root")]
    EscapesRoot { slug: Slug },

    #[error("{slug} is not valid UTF-8")]
    NotUtf8 { slug: Slug },

    #[error("could not parse {slug}: {source}")]
    Malformed {
        slug: Slug,
        #[source]
        source: PageError,
    },

    #[error(transparent)]
    Io(#[from] io::Error),
}

/// One page found while walking the wiki, without its contents.
///
/// The indexer compares `updated` and `size` against what it already has to
/// decide whether a page needs re-reading.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WalkEntry {
    pub slug: Slug,
    pub updated: DateTime<Utc>,
    pub size: u64,
}

/// A directory of markdown files.
#[derive(Debug, Clone)]
pub struct Store {
    root: PathBuf,
}

impl Store {
    /// Open (creating if necessary) the wiki directory at `root`.
    ///
    /// The root is canonicalised so that the containment check in
    /// [`Store::resolve`] compares like with like — on Windows that means both
    /// sides carry the `\\?\` verbatim prefix.
    pub async fn open(root: impl AsRef<Path>) -> Result<Self, StoreError> {
        let root = root.as_ref();
        tokio::fs::create_dir_all(root).await?;
        let root = tokio::fs::canonicalize(root).await?;
        Ok(Self { root })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The wiki root as it should be shown to a person.
    pub fn root_display(&self) -> String {
        display_path(&self.root)
    }

    /// The path a slug names, having confirmed it does not leave the wiki.
    ///
    /// A slug cannot traverse out on its own — that is settled in
    /// [`Slug::parse`] — but a symlink inside the wiki can point anywhere, so
    /// an existing target is canonicalised and checked. A path that does not
    /// exist yet has nothing to resolve and is returned as built.
    async fn resolve(&self, slug: &Slug) -> Result<PathBuf, StoreError> {
        let path = slug.to_path(&self.root);

        match tokio::fs::canonicalize(&path).await {
            Ok(canonical) if !canonical.starts_with(&self.root) => {
                Err(StoreError::EscapesRoot { slug: slug.clone() })
            }
            Ok(_) => Ok(path),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(path),
            Err(error) => Err(error.into()),
        }
    }

    pub async fn exists(&self, slug: &Slug) -> Result<bool, StoreError> {
        let path = self.resolve(slug).await?;
        Ok(tokio::fs::try_exists(&path).await?)
    }

    pub async fn read(&self, slug: &Slug) -> Result<Page, StoreError> {
        let path = self.resolve(slug).await?;

        let text = match tokio::fs::read_to_string(&path).await {
            Ok(text) => text,
            Err(error) => {
                return Err(match error.kind() {
                    io::ErrorKind::NotFound => StoreError::NotFound { slug: slug.clone() },
                    io::ErrorKind::InvalidData => StoreError::NotUtf8 { slug: slug.clone() },
                    _ => error.into(),
                });
            }
        };

        let updated = modified_at(&tokio::fs::metadata(&path).await?)?;

        Page::from_markdown(slug.clone(), &text, updated).map_err(|source| StoreError::Malformed {
            slug: slug.clone(),
            source,
        })
    }

    /// Write a page, replacing any existing one.
    pub async fn write(
        &self,
        slug: &Slug,
        frontmatter: Frontmatter,
        body: &str,
    ) -> Result<Page, StoreError> {
        let path = self.resolve(slug).await?;

        let page = Page {
            slug: slug.clone(),
            frontmatter,
            body: body.to_owned(),
            // Both replaced below with what the filesystem actually recorded.
            updated: Utc::now(),
            size: 0,
        };

        if let Some(parent) = path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        write_atomically(&path, page.to_markdown().as_bytes()).await?;

        let metadata = tokio::fs::metadata(&path).await?;
        Ok(Page {
            updated: modified_at(&metadata)?,
            size: metadata.len(),
            ..page
        })
    }

    /// Write a page that must not already exist.
    ///
    /// The check and the write are not atomic. Rhizolog is single-user, so the
    /// only way to lose that race is to race yourself.
    pub async fn create(
        &self,
        slug: &Slug,
        frontmatter: Frontmatter,
        body: &str,
    ) -> Result<Page, StoreError> {
        if self.exists(slug).await? {
            return Err(StoreError::AlreadyExists { slug: slug.clone() });
        }
        self.write(slug, frontmatter, body).await
    }

    pub async fn delete(&self, slug: &Slug) -> Result<(), StoreError> {
        let path = self.resolve(slug).await?;

        match tokio::fs::remove_file(&path).await {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(StoreError::NotFound { slug: slug.clone() });
            }
            Err(error) => return Err(error.into()),
        }

        self.prune_empty_parents(&path).await;
        Ok(())
    }

    /// Move a page to a new slug.
    ///
    /// Inbound links are deliberately left alone: they become wanted pages, so
    /// a move shows up in `/api/stats` instead of rotting quietly. See
    /// `knowledge-base/architecture.md`.
    pub async fn move_page(&self, from: &Slug, to: &Slug) -> Result<Page, StoreError> {
        if from == to {
            return self.read(from).await;
        }

        let source = self.resolve(from).await?;
        let destination = self.resolve(to).await?;

        if !tokio::fs::try_exists(&source).await? {
            return Err(StoreError::NotFound { slug: from.clone() });
        }
        if tokio::fs::try_exists(&destination).await? {
            return Err(StoreError::AlreadyExists { slug: to.clone() });
        }

        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        tokio::fs::rename(&source, &destination).await?;
        self.prune_empty_parents(&source).await;

        self.read(to).await
    }

    /// List every page in the wiki, in file-name order.
    ///
    /// The order is part of the contract. A directory lists its entries in
    /// whatever order the filesystem keeps, which is alphabetical on NTFS and
    /// hashed on ext4, and the startup scan writes a word-log baseline for each
    /// new page in the order it meets them. Unsorted, the same wiki scanned on
    /// Windows and on Linux wrote the same lines in a different order, which a
    /// test only found when CI first ran it on Linux.
    ///
    /// Blocking: callers on an async task should wrap this in
    /// `spawn_blocking`. It exists for the indexer's startup scan.
    pub fn walk(&self) -> Vec<WalkEntry> {
        let mut entries = Vec::new();

        let walker = WalkDir::new(&self.root)
            .follow_links(false)
            .sort_by_file_name()
            .into_iter()
            .filter_entry(|entry| {
                // Skip `.rhizolog`, `.git`, and anything else hidden by
                // convention. Slug validation rejects dot-segments too, so
                // these could never be addressed as pages anyway.
                entry.depth() == 0 || !file_name_starts_with_dot(entry.path())
            });

        for entry in walker {
            let entry = match entry {
                Ok(entry) => entry,
                Err(error) => {
                    tracing::warn!(%error, "skipping unreadable entry while walking the wiki");
                    continue;
                }
            };

            if !entry.file_type().is_file() {
                continue;
            }

            let Ok(relative) = entry.path().strip_prefix(&self.root) else {
                continue;
            };
            let Some(slug) = Slug::from_relative_path(relative) else {
                // Not a page: a stray `.txt`, an image, a name we would refuse
                // to create. Indexing it would produce a page we cannot serve.
                continue;
            };

            let metadata = match entry.metadata() {
                Ok(metadata) => metadata,
                Err(error) => {
                    tracing::warn!(%error, path = %entry.path().display(), "skipping unreadable page");
                    continue;
                }
            };
            let Ok(updated) = modified_at(&metadata) else {
                continue;
            };

            entries.push(WalkEntry {
                slug,
                updated,
                size: metadata.len(),
            });
        }

        entries
    }

    /// Remove directories left empty by a delete or a move, up to the root.
    ///
    /// Best-effort: a directory that is not empty ends the walk, and any other
    /// error is not worth failing an otherwise successful operation over.
    async fn prune_empty_parents(&self, path: &Path) {
        let mut current = path.parent();

        while let Some(directory) = current {
            if directory == self.root || !directory.starts_with(&self.root) {
                return;
            }
            if tokio::fs::remove_dir(directory).await.is_err() {
                return;
            }
            current = directory.parent();
        }
    }
}

fn file_name_starts_with_dot(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
}

/// A file's mtime, as the index records it.
pub(crate) fn modified_at(metadata: &std::fs::Metadata) -> io::Result<DateTime<Utc>> {
    Ok(metadata.modified()?.into())
}

/// Present a path for people to read.
///
/// The wiki root is canonicalised, and on Windows `canonicalize` returns a
/// verbatim path: `\\?\C:\wiki`. That prefix is correct, and it is also not
/// something anyone wants to see in an API response or a log line.
pub fn display_path(path: &Path) -> String {
    let text = path.display().to_string();

    // `\\?\UNC\server\share` is the verbatim spelling of `\\server\share`.
    if let Some(share) = text.strip_prefix(r"\\?\UNC\") {
        return format!(r"\\{share}");
    }
    text.strip_prefix(r"\\?\").unwrap_or(&text).to_owned()
}

/// Write via a temporary file in the same directory, then rename.
///
/// Same directory because a rename across volumes is not atomic; a leading dot
/// so that a temporary left behind by a crash is invisible to the walker.
pub(crate) async fn write_atomically(path: &Path, contents: &[u8]) -> io::Result<()> {
    let directory = path.parent().unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("page.md");
    let temporary = directory.join(format!(".{file_name}.tmp"));

    tokio::fs::write(&temporary, contents).await?;

    // `rename` replaces an existing destination on both Windows and Unix.
    if let Err(error) = tokio::fs::rename(&temporary, path).await {
        let _ = tokio::fs::remove_file(&temporary).await;
        return Err(error);
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    async fn store() -> (TempDir, Store) {
        let directory = TempDir::new().expect("temp dir");
        let store = Store::open(directory.path()).await.expect("open store");
        (directory, store)
    }

    fn slug(raw: &str) -> Slug {
        Slug::parse(raw).expect("valid slug")
    }

    fn frontmatter(title: &str) -> Frontmatter {
        Frontmatter {
            title: Some(title.to_owned()),
            ..Frontmatter::default()
        }
    }

    #[tokio::test]
    async fn writes_and_reads_a_page() {
        let (_directory, store) = store().await;
        let target = slug("notes/rhizome");

        let written = store
            .write(&target, frontmatter("Rhizome"), "Branches off.\n")
            .await
            .expect("write");
        assert_eq!(written.title(), "Rhizome");

        let read = store.read(&target).await.expect("read");
        assert_eq!(read.body, "Branches off.\n");
        assert_eq!(read.title(), "Rhizome");
    }

    #[tokio::test]
    async fn writing_creates_intermediate_directories() {
        let (directory, store) = store().await;

        store
            .write(&slug("a/b/c/deep"), Frontmatter::default(), "Deep.\n")
            .await
            .expect("write");

        assert!(directory.path().join("a/b/c/deep.md").is_file());
    }

    #[tokio::test]
    async fn reading_a_missing_page_reports_not_found() {
        let (_directory, store) = store().await;

        let error = store.read(&slug("nothing/here")).await.unwrap_err();
        assert!(matches!(error, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn create_refuses_to_clobber() {
        let (_directory, store) = store().await;
        let target = slug("notes/rhizome");

        store
            .create(&target, Frontmatter::default(), "First.\n")
            .await
            .expect("first create");
        let error = store
            .create(&target, Frontmatter::default(), "Second.\n")
            .await
            .unwrap_err();

        assert!(matches!(error, StoreError::AlreadyExists { .. }));
        // The failed create left the original alone.
        assert_eq!(store.read(&target).await.unwrap().body, "First.\n");
    }

    #[tokio::test]
    async fn write_replaces_an_existing_page() {
        let (_directory, store) = store().await;
        let target = slug("notes/rhizome");

        store
            .write(&target, Frontmatter::default(), "First.\n")
            .await
            .expect("first write");
        store
            .write(&target, Frontmatter::default(), "Second.\n")
            .await
            .expect("second write");

        assert_eq!(store.read(&target).await.unwrap().body, "Second.\n");
    }

    #[tokio::test]
    async fn deletes_a_page_and_prunes_the_directories_it_emptied() {
        let (directory, store) = store().await;
        let target = slug("a/b/c/deep");

        store
            .write(&target, Frontmatter::default(), "Deep.\n")
            .await
            .expect("write");
        store.delete(&target).await.expect("delete");

        assert!(
            !directory.path().join("a").exists(),
            "empty tree not pruned"
        );
        assert!(directory.path().exists(), "root must survive pruning");
    }

    #[tokio::test]
    async fn pruning_stops_at_a_directory_that_is_still_in_use() {
        let (directory, store) = store().await;

        store
            .write(&slug("a/keep"), Frontmatter::default(), "Keep.\n")
            .await
            .expect("write sibling");
        store
            .write(&slug("a/b/go"), Frontmatter::default(), "Go.\n")
            .await
            .expect("write nested");
        store.delete(&slug("a/b/go")).await.expect("delete");

        assert!(
            !directory.path().join("a/b").exists(),
            "empty dir not pruned"
        );
        assert!(directory.path().join("a/keep.md").is_file(), "sibling lost");
    }

    #[tokio::test]
    async fn deleting_a_missing_page_reports_not_found() {
        let (_directory, store) = store().await;

        let error = store.delete(&slug("nothing/here")).await.unwrap_err();
        assert!(matches!(error, StoreError::NotFound { .. }));
    }

    #[tokio::test]
    async fn moves_a_page() {
        let (_directory, store) = store().await;
        let from = slug("notes/old");
        let to = slug("archive/new");

        store
            .write(&from, frontmatter("Rhizome"), "Body.\n")
            .await
            .expect("write");
        let moved = store.move_page(&from, &to).await.expect("move");

        assert_eq!(moved.slug, to);
        assert_eq!(moved.body, "Body.\n");
        assert!(matches!(
            store.read(&from).await.unwrap_err(),
            StoreError::NotFound { .. }
        ));
    }

    #[tokio::test]
    async fn moving_onto_an_existing_page_is_refused() {
        let (_directory, store) = store().await;
        let from = slug("notes/old");
        let to = slug("notes/taken");

        store
            .write(&from, Frontmatter::default(), "Source.\n")
            .await
            .expect("write source");
        store
            .write(&to, Frontmatter::default(), "Destination.\n")
            .await
            .expect("write destination");

        let error = store.move_page(&from, &to).await.unwrap_err();
        assert!(matches!(error, StoreError::AlreadyExists { .. }));
        // Neither side moved.
        assert_eq!(store.read(&from).await.unwrap().body, "Source.\n");
        assert_eq!(store.read(&to).await.unwrap().body, "Destination.\n");
    }

    #[tokio::test]
    async fn walking_finds_every_page() {
        let (_directory, store) = store().await;
        for raw in ["index", "notes/rhizome", "notes/rust/async"] {
            store
                .write(&slug(raw), Frontmatter::default(), "Body.\n")
                .await
                .expect("write");
        }

        let mut found: Vec<String> = store
            .walk()
            .into_iter()
            .map(|entry| entry.slug.to_string())
            .collect();
        found.sort();

        assert_eq!(found, ["index", "notes/rhizome", "notes/rust/async"]);
    }

    /// In file-name order, whatever order the filesystem keeps, so that a scan
    /// does the same thing on every platform. NTFS happens to list names
    /// alphabetically, so on Windows this passes either way; it is Linux, in
    /// CI, that it is for.
    #[tokio::test]
    async fn walking_is_in_file_name_order() {
        let (_directory, store) = store().await;
        for raw in ["zebra", "notes/b", "apple", "notes/a", "mango"] {
            store
                .write(&slug(raw), Frontmatter::default(), "Body.\n")
                .await
                .expect("write");
        }

        let found: Vec<String> = store
            .walk()
            .into_iter()
            .map(|entry| entry.slug.to_string())
            .collect();

        assert_eq!(found, ["apple", "mango", "notes/a", "notes/b", "zebra"]);
    }

    #[tokio::test]
    async fn walking_skips_non_pages_and_hidden_directories() {
        let (directory, store) = store().await;
        store
            .write(&slug("real"), Frontmatter::default(), "Body.\n")
            .await
            .expect("write");

        tokio::fs::write(directory.path().join("notes.txt"), "not a page")
            .await
            .expect("write stray file");
        tokio::fs::write(directory.path().join("README"), "not a page")
            .await
            .expect("write extensionless file");
        tokio::fs::create_dir_all(directory.path().join(INTERNAL_DIR))
            .await
            .expect("create internal dir");
        tokio::fs::write(
            directory.path().join(INTERNAL_DIR).join("notes.md"),
            "internal",
        )
        .await
        .expect("write internal page");

        let found: Vec<String> = store
            .walk()
            .into_iter()
            .map(|entry| entry.slug.to_string())
            .collect();

        assert_eq!(found, ["real"]);
    }

    #[tokio::test]
    async fn walk_reports_size_and_mtime() {
        let (_directory, store) = store().await;
        let written = store
            .write(&slug("index"), Frontmatter::default(), "Body.\n")
            .await
            .expect("write");

        let entries = store.walk();
        let entry = entries.first().expect("one page");

        assert_eq!(entry.slug, written.slug);
        assert_eq!(entry.updated, written.updated);
        assert_eq!(entry.size, "Body.\n".len() as u64);
    }

    /// The temporary file a write goes through must never be visible as a page.
    #[tokio::test]
    async fn atomic_write_leaves_no_visible_temporary() {
        let (directory, store) = store().await;
        store
            .write(&slug("index"), Frontmatter::default(), "Body.\n")
            .await
            .expect("write");

        let mut names: Vec<String> = std::fs::read_dir(directory.path())
            .expect("read dir")
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();

        assert_eq!(names, ["index.md"]);
    }

    /// `canonicalize` hands back `\\?\C:\wiki` on Windows, and that prefix
    /// must not reach an API response.
    #[test]
    fn display_path_strips_the_windows_verbatim_prefix() {
        assert_eq!(display_path(Path::new(r"\\?\C:\wiki")), r"C:\wiki");
        assert_eq!(
            display_path(Path::new(r"\\?\UNC\server\share\wiki")),
            r"\\server\share\wiki"
        );
        // Ordinary paths pass through untouched.
        assert_eq!(display_path(Path::new(r"C:\wiki")), r"C:\wiki");
        assert_eq!(
            display_path(Path::new("/home/theaz/wiki")),
            "/home/theaz/wiki"
        );
    }

    #[tokio::test]
    async fn the_displayed_root_is_free_of_verbatim_prefixes() {
        let (_directory, store) = store().await;
        assert!(!store.root_display().starts_with(r"\\?\"));
    }

    #[tokio::test]
    async fn a_malformed_page_is_reported_against_its_slug() {
        let (directory, store) = store().await;
        tokio::fs::write(
            directory.path().join("broken.md"),
            "---\ntitle: never closed\n\nBody.\n",
        )
        .await
        .expect("write broken page");

        let error = store.read(&slug("broken")).await.unwrap_err();
        assert!(matches!(error, StoreError::Malformed { .. }));
    }
}
