//! What the app remembers between runs.
//!
//! Which wiki, and which port. There is no shell environment behind a
//! double-click, so something has to hold the answers the user gave last time —
//! and the `RHIZOLOG_*` variables still win over both, because somebody who
//! sets one means it.
//!
//! ## Beside the executable, then the config directory
//!
//! The file's first home is the directory the executable is in, because that is
//! what makes the app portable — copy that folder to a stick and the wiki it
//! points at goes with it. That only works where the directory can be written
//! to, which an install under `Program Files` cannot, so the OS config
//! directory is the fallback rather than a failure.
//!
//! Reading prefers whichever exists, in the same order. A portable copy beside
//! the executable therefore wins over a stale one left in `%APPDATA%` by an
//! earlier installed run, which is the way round anybody carrying the folder
//! around would expect.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

/// Unambiguous on purpose: beside a portable executable it may well be sharing
/// a directory with somebody else's files.
const FILE: &str = "rhizolog.settings.json";

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// The wiki to open. `None` until somebody has chosen one.
    pub wiki_root: Option<PathBuf>,
    /// The port to serve on, if one was asked for.
    ///
    /// `None` is the ordinary answer and means "negotiate": prefer 3000 and
    /// take any free port when it is busy. A number here is a requirement
    /// rather than a preference — see [`crate::listen_for`].
    ///
    /// Only the port. The host stays loopback, which is the whole security
    /// boundary and also the difference between starting quietly and raising a
    /// Windows Defender prompt.
    pub port: Option<u16>,
}

/// Read the settings, from wherever they turn out to be.
///
/// A file that cannot be read or parsed is reported as absent. The cost of
/// being wrong is one folder picker, and refusing to start because a remembered
/// preference is malformed would be a worse trade than asking again.
pub fn load(handle: &AppHandle) -> Settings {
    for path in candidates(handle) {
        if let Some(settings) = read(&path) {
            tracing::debug!(path = %path.display(), "read settings");
            return settings;
        }
    }
    Settings::default()
}

/// Write the settings to the first place that will take them.
pub fn save(handle: &AppHandle, settings: &Settings) -> anyhow::Result<()> {
    let mut last = None;

    for path in candidates(handle) {
        match write(&path, settings) {
            Ok(()) => {
                tracing::info!(path = %path.display(), "saved settings");
                return Ok(());
            }
            Err(error) => {
                tracing::debug!(path = %path.display(), %error, "could not save settings here");
                last =
                    Some(anyhow::Error::from(error).context(format!("writing {}", path.display())));
            }
        }
    }

    Err(last.unwrap_or_else(|| anyhow::anyhow!("nowhere to save settings")))
}

/// Where the settings might be, best first.
fn candidates(handle: &AppHandle) -> Vec<PathBuf> {
    let mut paths = Vec::new();

    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        paths.push(directory.join(FILE));
    }

    if let Ok(directory) = handle.path().app_config_dir() {
        paths.push(directory.join(FILE));
    }

    paths
}

fn read(path: &Path) -> Option<Settings> {
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(strip_bom(&bytes)).ok()
}

/// Drop a UTF-8 byte order mark, if there is one.
///
/// This is a small text file in a folder people are invited to carry around, so
/// it will get hand-edited — and on Windows, Notepad and PowerShell's
/// `Set-Content -Encoding utf8` both write a BOM without being asked. A JSON
/// parser rejects the document outright, so without this the app quietly
/// forgets which wiki it opens and asks again, which is a baffling thing for
/// fixing a typo to do.
///
/// The same hazard, for the same reason, as the one page parsing already
/// handles — see the BOM section of `knowledge-base/architecture.md`.
fn strip_bom(bytes: &[u8]) -> &[u8] {
    bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
}

fn write(path: &Path, settings: &Settings) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(settings)?)
}

#[cfg(test)]
mod tests {
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn settings_round_trip() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join(FILE);
        let settings = Settings {
            wiki_root: Some(PathBuf::from("/somewhere/wiki")),
            port: Some(8080),
        };

        write(&path, &settings).expect("write");

        assert_eq!(read(&path), Some(settings));
    }

    #[test]
    fn a_wiki_that_was_never_chosen_round_trips_as_absent() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join(FILE);

        write(&path, &Settings::default()).expect("write");

        assert_eq!(read(&path), Some(Settings::default()));
    }

    /// Missing and malformed are the same answer, because the response to both
    /// is to ask again rather than to fail.
    #[test]
    fn an_unreadable_file_reads_as_nothing_remembered() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join(FILE);

        assert_eq!(read(&path), None, "a file that is not there");

        std::fs::write(&path, "{ not json").expect("write rubbish");
        assert_eq!(read(&path), None, "a file that is not settings");
    }

    /// Fields added later must not make an older file unreadable — the whole
    /// point of it is to survive between versions.
    #[test]
    fn an_unknown_field_does_not_invalidate_the_file() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join(FILE);

        std::fs::write(
            &path,
            r#"{"wiki_root": "/somewhere/wiki", "window_width": 900}"#,
        )
        .expect("write");

        assert_eq!(
            read(&path),
            Some(Settings {
                wiki_root: Some(PathBuf::from("/somewhere/wiki")),
                port: None,
            })
        );
    }

    /// The other direction, and the one that actually happened: a file written
    /// before there was a port setting has to keep opening the same wiki rather
    /// than sending its owner back to the folder picker.
    #[test]
    fn a_file_from_before_the_port_existed_still_reads() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join(FILE);

        std::fs::write(&path, r#"{"wiki_root": "/somewhere/wiki"}"#).expect("write");

        assert_eq!(
            read(&path),
            Some(Settings {
                wiki_root: Some(PathBuf::from("/somewhere/wiki")),
                port: None,
            })
        );
    }

    /// Found by writing the file from PowerShell, which is how anybody on this
    /// platform would: `Set-Content -Encoding utf8` prepends a BOM, the JSON
    /// parser refuses the document, and the app silently asks which wiki to
    /// open again. Notepad does the same thing.
    #[test]
    fn a_utf8_bom_does_not_hide_the_settings() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join(FILE);
        let settings = Settings {
            wiki_root: Some(PathBuf::from("/somewhere/wiki")),
            port: None,
        };

        let mut bytes = vec![0xEF, 0xBB, 0xBF];
        bytes.extend_from_slice(&serde_json::to_vec(&settings).expect("serialize"));
        std::fs::write(&path, bytes).expect("write with a BOM");

        assert_eq!(read(&path), Some(settings));
    }

    #[test]
    fn writing_creates_the_directory_it_needs() {
        let directory = TempDir::new().expect("temp dir");
        let path = directory.path().join("never-made").join(FILE);

        write(&path, &Settings::default()).expect("write");

        assert!(path.exists());
    }
}
