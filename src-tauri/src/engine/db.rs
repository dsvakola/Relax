//! Sync database: remembers the state of every item after the last successful sync of a folder pair.
//!
//! Two-way sync needs it to tell "deleted on the right" apart from "new on the left", and
//! "changed on one side" apart from "changed on both sides". It is stored in Relax's own data folder,
//! never inside the user's folders.

use super::compare::{Category, FolderPairResult};
use super::scan::{sort_key, ItemKind};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct DbEntry {
    pub kind: ItemKind,
    pub size: u64,
    /// Modification time (100 ns ticks) on each side right after the last sync.
    pub left_modified: i64,
    pub right_modified: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SyncDb {
    version: u32,
    /// Keyed by [`sort_key`] of the relative path.
    entries: HashMap<String, DbEntry>,
}

impl SyncDb {
    pub fn get(&self, rel_path: &str) -> Option<&DbEntry> {
        self.entries.get(&sort_key(rel_path))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Records every item that is identical on both sides. Call this on a fresh comparison
    /// made right after a sync finished.
    pub fn from_comparison(result: &FolderPairResult) -> Self {
        let entries = result
            .pairs
            .iter()
            .filter(|p| p.category == Category::Equal)
            .filter_map(|p| {
                let (l, r) = (p.left.as_ref()?, p.right.as_ref()?);
                Some((
                    sort_key(&p.rel_path),
                    DbEntry {
                        kind: l.kind,
                        size: l.size,
                        left_modified: l.modified,
                        right_modified: r.modified,
                    },
                ))
            })
            .collect();
        Self {
            version: FORMAT_VERSION,
            entries,
        }
    }

    /// Database to store after a sync. `now` is a fresh comparison made right after the sync.
    /// Items that are equal now are recorded with their new state. Items that still differ (because
    /// the sync failed or was cancelled for them) keep their old record, so a pending deletion is not
    /// mistaken for a new file next time. Items gone from both sides are forgotten.
    pub fn after_sync(old: Option<&SyncDb>, now: &FolderPairResult) -> Self {
        let mut db = Self::from_comparison(now);
        if let Some(old) = old {
            for p in now.pairs.iter().filter(|p| p.category != Category::Equal) {
                let key = sort_key(&p.rel_path);
                if let Some(e) = old.entries.get(&key) {
                    db.entries.insert(key, *e);
                }
            }
        }
        db
    }

    /// File that holds the database of one folder pair inside `dir` (Relax's data folder).
    pub fn file_for_pair(dir: &Path, left: &str, right: &str) -> PathBuf {
        let key = format!("{}|{}", normalize(left), normalize(right));
        dir.join(format!("pair-{:016x}.json", fnv1a(key.as_bytes())))
    }

    /// Loads the database, or `None` if there is none yet (first sync) or it is unreadable.
    pub fn load(file: &Path) -> Option<Self> {
        let text = fs::read_to_string(file).ok()?;
        let db: SyncDb = serde_json::from_str(&text).ok()?;
        (db.version == FORMAT_VERSION).then_some(db)
    }

    /// Saves atomically: writes a temporary file, flushes it to disk, then renames it over the old one.
    pub fn save(&self, file: &Path) -> std::io::Result<()> {
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = file.with_extension("json.tmp");
        {
            let mut f = fs::File::create(&tmp)?;
            serde_json::to_writer(&mut f, self).map_err(std::io::Error::other)?;
            f.flush()?;
            f.sync_all()?;
        }
        fs::rename(&tmp, file)
    }
}

fn normalize(path: &str) -> String {
    path.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Small stable hash (FNV-1a, 64-bit) for file names; not used for security.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_name_ignores_case_and_trailing_slash() {
        let d = Path::new(r"C:\data");
        assert_eq!(
            SyncDb::file_for_pair(d, r"D:\Work\", "F:/Backup"),
            SyncDb::file_for_pair(d, r"d:\work", r"F:\BACKUP")
        );
        assert_ne!(
            SyncDb::file_for_pair(d, r"D:\Work", r"F:\Backup"),
            SyncDb::file_for_pair(d, r"F:\Backup", r"D:\Work")
        );
    }

    #[test]
    fn save_and_load_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut db = SyncDb {
            version: FORMAT_VERSION,
            ..Default::default()
        };
        db.entries.insert(
            sort_key(r"a\b.txt"),
            DbEntry {
                kind: ItemKind::File,
                size: 5,
                left_modified: 10,
                right_modified: 11,
            },
        );
        let file = tmp.path().join("sub").join("pair.json");
        db.save(&file).unwrap();
        let loaded = SyncDb::load(&file).unwrap();
        assert_eq!(loaded, db);
        assert_eq!(loaded.get(r"A\B.TXT").unwrap().size, 5);
        assert!(!file.with_extension("json.tmp").exists());
    }

    #[test]
    fn after_sync_keeps_records_of_unfinished_items_only() {
        use crate::engine::compare::{CompareStats, FilePair};
        use crate::engine::scan::FsItem;
        let file = |rel: &str, size: u64| FsItem {
            rel_path: rel.into(),
            kind: ItemKind::File,
            size,
            modified: 0,
        };
        let entry = |size: u64| DbEntry {
            kind: ItemKind::File,
            size,
            left_modified: 0,
            right_modified: 0,
        };
        let mut old = SyncDb {
            version: FORMAT_VERSION,
            ..Default::default()
        };
        old.entries.insert(sort_key("pending_delete.txt"), entry(1));
        old.entries.insert(sort_key("gone_everywhere.txt"), entry(1));
        old.entries.insert(sort_key("synced.txt"), entry(1));
        let now = FolderPairResult {
            left: "L".into(),
            right: "R".into(),
            left_exists: true,
            right_exists: true,
            pairs: vec![
                FilePair {
                    rel_path: "pending_delete.txt".into(),
                    left: Some(file("pending_delete.txt", 1)),
                    right: None,
                    category: Category::LeftOnly,
                    conflict_reason: None,
                },
                FilePair {
                    rel_path: "synced.txt".into(),
                    left: Some(file("synced.txt", 2)),
                    right: Some(file("synced.txt", 2)),
                    category: Category::Equal,
                    conflict_reason: None,
                },
            ],
            stats: CompareStats::default(),
            errors: vec![],
            skipped_links: vec![],
        };
        let db = SyncDb::after_sync(Some(&old), &now);
        assert_eq!(db.get("pending_delete.txt").unwrap().size, 1);
        assert_eq!(db.get("synced.txt").unwrap().size, 2);
        assert!(db.get("gone_everywhere.txt").is_none());
    }

    #[test]
    fn missing_or_corrupt_database_loads_as_none() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("pair.json");
        assert!(SyncDb::load(&file).is_none());
        fs::write(&file, "{ not json").unwrap();
        assert!(SyncDb::load(&file).is_none());
    }
}
