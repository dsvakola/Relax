//! Folder scanner: walks one base folder and records every file and sub-folder.
//!
//! - Uses the directory listing data Windows already returns, so no file is opened.
//! - Long paths (over 260 characters) and Unicode names work (Rust adds the `\\?\` prefix itself).
//! - Symbolic links and junctions are listed as skipped and never followed, so loops are impossible.
//! - An unreadable sub-folder or file is recorded as an [`ItemError`]; the scan continues.

use super::filter::Filter;
use super::{CancelToken, EngineError, ItemError, Side};
use serde::{Serialize, Serializer};
use std::collections::BTreeMap;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

/// Time stamps are kept as 100-nanosecond ticks since 1970 (Windows' own precision).
pub const TICKS_PER_SECOND: i64 = 10_000_000;

/// Folder at a drive root where Relax keeps replaced/deleted files when no Recycle Bin is available.
pub const VERSIONS_DIR: &str = "Relax-Versions";

/// Suffix of the temporary file a copy is written to before it is renamed into place.
/// Such files are leftovers of an interrupted sync and are never synced themselves.
pub const TEMP_SUFFIX: &str = ".relax-tmp";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ItemKind {
    File,
    Folder,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsItem {
    /// Path relative to the base folder, `\`-separated, exactly as spelled on disk.
    pub rel_path: String,
    pub kind: ItemKind,
    /// File size in bytes (0 for folders).
    pub size: u64,
    /// Last modification time in 100 ns ticks since 1970. Sent to the UI as milliseconds.
    #[serde(rename = "modifiedMs", serialize_with = "ticks_as_ms")]
    pub modified: i64,
}

fn ticks_as_ms<S: Serializer>(ticks: &i64, s: S) -> Result<S::Ok, S::Error> {
    s.serialize_i64(ticks.div_euclid(10_000))
}

#[derive(Debug, Default)]
pub struct ScanOutput {
    /// Items keyed by [`sort_key`], so a folder is followed directly by its contents.
    pub items: BTreeMap<String, FsItem>,
    pub errors: Vec<ItemError>,
    pub files: u64,
    pub folders: u64,
    pub bytes: u64,
    /// Symbolic links / junctions that were not followed.
    pub skipped_links: Vec<String>,
}

/// Case-insensitive key that sorts children directly after their parent folder.
pub fn sort_key(rel_path: &str) -> String {
    rel_path.to_lowercase().replace('\\', "\u{0}")
}

pub fn system_time_to_ticks(t: SystemTime) -> i64 {
    match t.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs() as i64 * TICKS_PER_SECOND + (d.subsec_nanos() / 100) as i64,
        Err(e) => {
            let d = e.duration();
            -(d.as_secs() as i64 * TICKS_PER_SECOND + (d.subsec_nanos() / 100) as i64)
        }
    }
}

/// Scans `base`. A missing base folder is reported as `Ok(None)`; the caller decides what that means.
/// `on_progress` receives the running item count roughly every 256 items.
pub fn scan_folder(
    base: &Path,
    side: Side,
    filter: &Filter,
    cancel: &CancelToken,
    on_progress: &dyn Fn(u64),
) -> Result<Option<ScanOutput>, EngineError> {
    match fs::metadata(base) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(EngineError::CannotReadFolder {
                path: base.display().to_string(),
                message: e.to_string(),
            })
        }
        Ok(m) if !m.is_dir() => {
            return Err(EngineError::NotAFolder {
                path: base.display().to_string(),
            })
        }
        Ok(_) => {}
    }
    // The base folder itself must be listable; otherwise the result would look like an empty folder.
    if let Err(e) = fs::read_dir(base) {
        return Err(EngineError::CannotReadFolder {
            path: base.display().to_string(),
            message: e.to_string(),
        });
    }

    // Relax's own fallback version folder at a drive root is never part of a sync.
    let base_is_root = base.parent().is_none();
    let mut out = ScanOutput::default();
    // Stack of relative folder paths still to list ("" = base folder).
    let mut pending: Vec<String> = vec![String::new()];
    let mut counter: u64 = 0;

    while let Some(rel_dir) = pending.pop() {
        if cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        let abs_dir = if rel_dir.is_empty() {
            base.to_path_buf()
        } else {
            base.join(&rel_dir)
        };
        let entries = match fs::read_dir(&abs_dir) {
            Ok(e) => e,
            Err(e) => {
                out.errors.push(ItemError {
                    side,
                    path: abs_dir.display().to_string(),
                    message: e.to_string(),
                });
                continue;
            }
        };

        for entry in entries {
            let entry = match entry {
                Ok(e) => e,
                Err(e) => {
                    out.errors.push(ItemError {
                        side,
                        path: abs_dir.display().to_string(),
                        message: e.to_string(),
                    });
                    continue;
                }
            };
            let name = entry.file_name().to_string_lossy().into_owned();
            let rel_path = if rel_dir.is_empty() {
                name
            } else {
                format!("{rel_dir}\\{}", name)
            };

            let file_type = match entry.file_type() {
                Ok(t) => t,
                Err(e) => {
                    out.errors.push(ItemError {
                        side,
                        path: entry.path().display().to_string(),
                        message: e.to_string(),
                    });
                    continue;
                }
            };

            if file_type.is_symlink() {
                if !filter.excludes_folder(&rel_path) {
                    out.skipped_links.push(rel_path);
                }
                continue;
            }

            if file_type.is_dir() {
                if filter.excludes_folder(&rel_path) || (base_is_root && rel_path.eq_ignore_ascii_case(VERSIONS_DIR)) {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .map(system_time_to_ticks)
                    .unwrap_or(0);
                out.folders += 1;
                pending.push(rel_path.clone());
                out.items.insert(
                    sort_key(&rel_path),
                    FsItem {
                        rel_path,
                        kind: ItemKind::Folder,
                        size: 0,
                        modified,
                    },
                );
            } else {
                if !filter.keeps_file(&rel_path) || rel_path.to_ascii_lowercase().ends_with(TEMP_SUFFIX) {
                    continue;
                }
                let meta = match entry.metadata() {
                    Ok(m) => m,
                    Err(e) => {
                        out.errors.push(ItemError {
                            side,
                            path: entry.path().display().to_string(),
                            message: e.to_string(),
                        });
                        continue;
                    }
                };
                let modified = meta.modified().map(system_time_to_ticks).unwrap_or(0);
                out.files += 1;
                out.bytes += meta.len();
                out.items.insert(
                    sort_key(&rel_path),
                    FsItem {
                        rel_path,
                        kind: ItemKind::File,
                        size: meta.len(),
                        modified,
                    },
                );
            }

            counter += 1;
            if counter.is_multiple_of(256) {
                on_progress(counter);
                if cancel.is_cancelled() {
                    return Err(EngineError::Cancelled);
                }
            }
        }
    }

    // Folders that only existed to hold now-filtered files stay listed; that matches FreeFileSync.
    on_progress(counter);
    Ok(Some(out))
}
