//! Executor: carries out a reviewed sync plan on disk, as safely as possible.
//!
//! - Every file is first copied to `name.relax-tmp`, optionally verified, and only then renamed into
//!   place. A power cut or unplugged drive never leaves a half-written file under the real name.
//! - Files that are deleted or overwritten go to the Recycle Bin (default), a versions folder, or are
//!   deleted permanently, as the user chose. Drives without a Recycle Bin (USB sticks, memory cards,
//!   network drives) automatically use `<drive>\Relax-Versions\<date time>\...` instead.
//! - Before an item is overwritten or deleted, it is checked to be unchanged since the comparison.
//! - Free space is checked before anything starts.
//! - Deletions run first (deepest items first, freeing space); then folders and files are created
//!   in order, parents before children. Folders are only removed when empty.
//! - Pause, cancel, and Retry / Ignore / Ignore all / Cancel on every error.

use super::compare::files_identical;
use super::plan::{Action, PairPlan, PlannedItem};
use super::scan::{system_time_to_ticks, FsItem, ItemKind, TEMP_SUFFIX, TICKS_PER_SECOND, VERSIONS_DIR};
use super::{winfs, CancelToken, EngineError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum DeletionPolicy {
    /// Windows Recycle Bin; falls back to `<drive>\Relax-Versions` where there is none.
    #[default]
    RecycleBin,
    /// Move to `<folder>\<date time>\<relative path>`.
    Versioning { folder: String },
    /// Delete for good.
    Permanent,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecOptions {
    pub deletion: DeletionPolicy,
    /// Re-read every copied file and compare it byte by byte with the source.
    pub verify_copies: bool,
}

/// Lets the UI pause and resume a running sync.
#[derive(Clone, Default, Debug)]
pub struct PauseToken(Arc<(Mutex<bool>, Condvar)>);

impl PauseToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_paused(&self, paused: bool) {
        let (lock, cv) = &*self.0;
        *lock.lock().unwrap_or_else(|e| e.into_inner()) = paused;
        cv.notify_all();
    }

    pub fn is_paused(&self) -> bool {
        *self.0 .0.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Blocks while paused. Returns early when cancelled.
    fn wait(&self, cancel: &CancelToken) {
        let (lock, cv) = &*self.0;
        let mut paused = lock.lock().unwrap_or_else(|e| e.into_inner());
        while *paused && !cancel.is_cancelled() {
            paused = cv
                .wait_timeout(paused, Duration::from_millis(200))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemFailure {
    pub rel_path: String,
    pub action: Action,
    pub message: String,
}

/// The user's answer when an item fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorDecision {
    Retry,
    Ignore,
    IgnoreAll,
    Cancel,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecProgress {
    pub items_done: u64,
    pub items_total: u64,
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub current_path: String,
    pub current_action: Action,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionCounts {
    pub create_left: u64,
    pub create_right: u64,
    pub update_left: u64,
    pub update_right: u64,
    pub delete_left: u64,
    pub delete_right: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExecReport {
    pub left: String,
    pub right: String,
    pub done: ActionCounts,
    pub bytes_copied: u64,
    pub failures: Vec<ItemFailure>,
    /// Information for the log, e.g. where replaced files were kept.
    pub notes: Vec<String>,
    pub cancelled: bool,
    pub duration_ms: u64,
}

/// Carries out `plan`. `free_space` reports free bytes for a path (normally [`winfs::free_space`]).
pub fn execute(
    plan: &PairPlan,
    options: &ExecOptions,
    cancel: &CancelToken,
    pause: &PauseToken,
    free_space: &dyn Fn(&Path) -> Option<u64>,
    on_progress: &dyn Fn(&ExecProgress),
    on_error: &dyn Fn(&ItemFailure) -> ErrorDecision,
) -> Result<ExecReport, EngineError> {
    if let Some(reason) = &plan.blocked {
        return Err(EngineError::Blocked(reason.clone()));
    }
    let left_base = PathBuf::from(plan.left.trim());
    let right_base = PathBuf::from(plan.right.trim());

    for (base, needed) in [
        (&left_base, plan.stats.left.bytes_to_write),
        (&right_base, plan.stats.right.bytes_to_write),
    ] {
        if needed == 0 {
            continue;
        }
        if let Some(available) = free_space(base) {
            if needed > available {
                return Err(EngineError::NotEnoughSpace {
                    path: base.display().to_string(),
                    needed,
                    available,
                });
            }
        }
    }

    let active = |a: Action| !matches!(a, Action::None | Action::Conflict);
    let mut ex = Executor {
        left_base,
        right_base,
        options,
        cancel,
        pause,
        on_progress,
        on_error,
        timestamp: winfs::local_timestamp(),
        recycle_cache: HashMap::new(),
        ignore_all: false,
        last_emit: Instant::now() - Duration::from_secs(1),
        progress: ExecProgress {
            items_done: 0,
            items_total: plan.items.iter().filter(|i| active(i.action)).count() as u64,
            bytes_done: 0,
            bytes_total: plan.stats.left.bytes_to_write + plan.stats.right.bytes_to_write,
            current_path: String::new(),
            current_action: Action::None,
        },
        report: ExecReport {
            left: plan.left.clone(),
            right: plan.right.clone(),
            ..Default::default()
        },
    };
    let started = Instant::now();

    // Phase 1: deletions, deepest first. Phase 2: creations and updates, parents first.
    let deletions = plan
        .items
        .iter()
        .rev()
        .filter(|i| matches!(i.action, Action::DeleteLeft | Action::DeleteRight));
    let writes = plan.items.iter().filter(|i| {
        matches!(
            i.action,
            Action::CreateLeft | Action::CreateRight | Action::UpdateLeft | Action::UpdateRight
        )
    });
    let outcome = deletions.chain(writes).try_for_each(|item| ex.run_item(item));

    ex.report.duration_ms = started.elapsed().as_millis() as u64;
    ex.report.bytes_copied = ex.progress.bytes_done;
    match outcome {
        Ok(()) => Ok(ex.report),
        Err(EngineError::Cancelled) => {
            ex.report.cancelled = true;
            Ok(ex.report)
        }
        Err(e) => Err(e),
    }
}

struct Executor<'a> {
    left_base: PathBuf,
    right_base: PathBuf,
    options: &'a ExecOptions,
    cancel: &'a CancelToken,
    pause: &'a PauseToken,
    on_progress: &'a dyn Fn(&ExecProgress),
    on_error: &'a dyn Fn(&ItemFailure) -> ErrorDecision,
    timestamp: String,
    recycle_cache: HashMap<PathBuf, bool>,
    ignore_all: bool,
    last_emit: Instant,
    progress: ExecProgress,
    report: ExecReport,
}

impl Executor<'_> {
    fn emit(&mut self, force: bool) {
        if force || self.last_emit.elapsed() >= Duration::from_millis(100) {
            self.last_emit = Instant::now();
            (self.on_progress)(&self.progress);
        }
    }

    fn run_item(&mut self, item: &PlannedItem) -> Result<(), EngineError> {
        self.pause.wait(self.cancel);
        if self.cancel.is_cancelled() {
            return Err(EngineError::Cancelled);
        }
        self.progress.current_path = item.pair.rel_path.clone();
        self.progress.current_action = item.action;
        self.emit(true);

        let bytes_before = self.progress.bytes_done;
        loop {
            match self.do_item(item) {
                Ok(()) => {
                    self.count(item.action);
                    break;
                }
                Err(_) if self.cancel.is_cancelled() => {
                    // A cancelled partial copy was removed, so its bytes do not count.
                    self.progress.bytes_done = bytes_before;
                    return Err(EngineError::Cancelled);
                }
                Err(e) => {
                    // Bytes of a failed attempt do not count as copied.
                    self.progress.bytes_done = bytes_before;
                    let failure = ItemFailure {
                        rel_path: item.pair.rel_path.clone(),
                        action: item.action,
                        message: e.to_string(),
                    };
                    let decision = if self.ignore_all {
                        ErrorDecision::Ignore
                    } else {
                        (self.on_error)(&failure)
                    };
                    match decision {
                        ErrorDecision::Retry => continue,
                        ErrorDecision::Ignore => self.report.failures.push(failure),
                        ErrorDecision::IgnoreAll => {
                            self.ignore_all = true;
                            self.report.failures.push(failure);
                        }
                        ErrorDecision::Cancel => {
                            self.report.failures.push(failure);
                            self.cancel.cancel();
                            return Err(EngineError::Cancelled);
                        }
                    }
                    break;
                }
            }
        }
        self.progress.items_done += 1;
        self.emit(true);
        Ok(())
    }

    fn count(&mut self, a: Action) {
        let c = &mut self.report.done;
        match a {
            Action::CreateLeft => c.create_left += 1,
            Action::CreateRight => c.create_right += 1,
            Action::UpdateLeft => c.update_left += 1,
            Action::UpdateRight => c.update_right += 1,
            Action::DeleteLeft => c.delete_left += 1,
            Action::DeleteRight => c.delete_right += 1,
            Action::None | Action::Conflict => {}
        }
    }

    fn do_item(&mut self, item: &PlannedItem) -> io::Result<()> {
        let rel = &item.pair.rel_path;
        let p = &item.pair;
        match item.action {
            Action::DeleteLeft => self.delete(&self.left_base.clone(), rel, p.left.as_ref()),
            Action::DeleteRight => self.delete(&self.right_base.clone(), rel, p.right.as_ref()),
            Action::CreateRight | Action::UpdateRight => {
                let (src, dst) = (self.left_base.clone(), self.right_base.clone());
                self.write(&src, &dst, rel, p.left.as_ref(), p.right.as_ref())
            }
            Action::CreateLeft | Action::UpdateLeft => {
                let (src, dst) = (self.right_base.clone(), self.left_base.clone());
                self.write(&src, &dst, rel, p.right.as_ref(), p.left.as_ref())
            }
            Action::None | Action::Conflict => Ok(()),
        }
    }

    fn delete(&mut self, base: &Path, rel: &str, expected: Option<&FsItem>) -> io::Result<()> {
        let path = base.join(rel);
        let Some(expected) = expected else { return Ok(()) };
        match expected.kind {
            ItemKind::Folder => {
                if !path.exists() {
                    return Ok(());
                }
                // Only empty folders are removed; anything left inside was excluded by the filter.
                if fs::read_dir(&path)?.next().is_some() {
                    self.report.notes.push(format!(
                        "Kept folder {} because it still contains items excluded from the sync.",
                        path.display()
                    ));
                    return Ok(());
                }
                fs::remove_dir(&path)
            }
            ItemKind::File => {
                if !path.exists() {
                    return Ok(());
                }
                ensure_unchanged(&path, expected)?;
                self.retire(base, rel, &path)
            }
        }
    }

    fn write(
        &mut self,
        src_base: &Path,
        dst_base: &Path,
        rel: &str,
        src_item: Option<&FsItem>,
        dst_item: Option<&FsItem>,
    ) -> io::Result<()> {
        let src = src_base.join(rel);
        let dst = dst_base.join(rel);
        let Some(src_item) = src_item else { return Ok(()) };

        // If the target currently has the other type (file vs. folder), remove it first.
        if let Some(d) = dst_item {
            if d.kind != src_item.kind && fs::symlink_metadata(&dst).is_ok() {
                match d.kind {
                    ItemKind::File => {
                        ensure_unchanged(&dst, d)?;
                        self.retire(dst_base, rel, &dst)?;
                    }
                    ItemKind::Folder => {
                        if fs::read_dir(&dst)?.next().is_some() {
                            return Err(io::Error::other(
                                "A folder with this name is in the way and still contains files excluded from the sync.",
                            ));
                        }
                        fs::remove_dir(&dst)?;
                    }
                }
            }
        }

        if src_item.kind == ItemKind::Folder {
            return fs::create_dir_all(&dst);
        }

        ensure_unchanged(&src, src_item)?;
        if let Some(parent) = dst.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut tmp_name = dst.file_name().unwrap_or_default().to_os_string();
        tmp_name.push(TEMP_SUFFIX);
        let tmp = dst.with_file_name(tmp_name);
        if fs::symlink_metadata(&tmp).is_ok() {
            remove_file_forced(&tmp)?;
        }

        let cancel = self.cancel.clone();
        let pause = self.pause.clone();
        {
            let mut on_chunk = |n: u64| {
                pause.wait(&cancel);
                if cancel.is_cancelled() {
                    return false;
                }
                self.progress.bytes_done += n;
                self.emit(false);
                true
            };
            winfs::copy_file(&src, &tmp, &mut on_chunk)?;
        }

        if self.options.verify_copies {
            let same = files_identical(&src, &tmp, self.cancel, &mut |_| {});
            if !matches!(same, Ok(true)) {
                let _ = remove_file_forced(&tmp);
                return Err(match same {
                    Err(e) => e,
                    _ => io::Error::other("Verification failed: the copy is not identical to the original."),
                });
            }
        }

        if fs::symlink_metadata(&dst).is_ok() {
            let retired = match dst_item {
                Some(d) if d.kind == ItemKind::File => {
                    ensure_unchanged(&dst, d).and_then(|_| self.retire(dst_base, rel, &dst))
                }
                // A file appeared there after the comparison: do not touch it.
                _ => Err(io::Error::other(
                    "A file appeared at the target after the comparison. Please compare again.",
                )),
            };
            if let Err(e) = retired {
                let _ = remove_file_forced(&tmp);
                return Err(e);
            }
        }
        fs::rename(&tmp, &dst).map_err(|e| {
            io::Error::other(format!(
                "The old file was set aside, but the new copy could not be renamed into place: {e}. The new copy is at {}",
                tmp.display()
            ))
        })
    }

    /// Removes a file according to the deletion policy (Recycle Bin, versions folder or permanently).
    fn retire(&mut self, base: &Path, rel: &str, path: &Path) -> io::Result<()> {
        match &self.options.deletion {
            DeletionPolicy::Permanent => remove_file_forced(path),
            DeletionPolicy::Versioning { folder } => {
                let target = Path::new(folder).join(&self.timestamp).join(rel);
                move_file(path, &target)
            }
            DeletionPolicy::RecycleBin => {
                let root = winfs::drive_root(base).unwrap_or_else(|| base.to_path_buf());
                let has_bin = *self
                    .recycle_cache
                    .entry(root.clone())
                    .or_insert_with(|| winfs::has_recycle_bin(&root));
                if has_bin && winfs::move_to_recycle_bin(path).is_ok() {
                    return Ok(());
                }
                // No Recycle Bin here (or the path is too long for it): keep a copy on the same drive.
                let target = auto_versions_path(path, &self.timestamp)
                    .ok_or_else(|| io::Error::other("Cannot determine the drive of this file."))?;
                let versions_root = root.join(VERSIONS_DIR).join(&self.timestamp);
                let note = format!(
                    "Replaced and deleted files on {} were kept in {} (no Recycle Bin on this drive).",
                    root.display(),
                    versions_root.display()
                );
                if !self.report.notes.contains(&note) {
                    self.report.notes.push(note);
                }
                move_file(path, &target)
            }
        }
    }
}

/// `<drive>\Relax-Versions\<timestamp>\<path without drive>`, e.g.
/// `F:\Backup\a.txt` -> `F:\Relax-Versions\2026-10-19 083512\Backup\a.txt`.
pub fn auto_versions_path(path: &Path, timestamp: &str) -> Option<PathBuf> {
    let root = winfs::drive_root(path)?;
    let abs = std::path::absolute(path).ok()?;
    let rest = abs.strip_prefix(&root).ok()?;
    Some(root.join(VERSIONS_DIR).join(timestamp).join(rest))
}

/// Fails if the item on disk no longer matches what the comparison saw (someone changed it meanwhile).
fn ensure_unchanged(path: &Path, expected: &FsItem) -> io::Result<()> {
    let meta = fs::metadata(path)?;
    let modified = meta.modified().map(system_time_to_ticks).unwrap_or(0);
    if meta.len() != expected.size || (modified - expected.modified).abs() > TICKS_PER_SECOND {
        return Err(io::Error::other(
            "This file changed after the comparison. Please compare again before synchronizing.",
        ));
    }
    Ok(())
}

/// Deletes a file even if it is marked read-only.
fn remove_file_forced(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => {
            let mut perms = fs::metadata(path)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            perms.set_readonly(false);
            fs::set_permissions(path, perms)?;
            fs::remove_file(path)
        }
        other => other,
    }
}

/// Moves a file, also across drives. Never overwrites: adds " (2)", " (3)" ... if needed.
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if let Some(parent) = to.parent() {
        fs::create_dir_all(parent)?;
    }
    let mut target = to.to_path_buf();
    let mut n = 2;
    while fs::symlink_metadata(&target).is_ok() {
        let stem = to.file_stem().unwrap_or_default().to_string_lossy();
        let ext = to
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        target = to.with_file_name(format!("{stem} ({n}){ext}"));
        n += 1;
    }
    match fs::rename(from, &target) {
        Ok(()) => Ok(()),
        // ERROR_NOT_SAME_DEVICE: copy, then delete the original.
        Err(e) if e.raw_os_error() == Some(17) => {
            winfs::copy_file(from, &target, &mut |_| true)?;
            remove_file_forced(from)
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compare::{compare, Category, CompareOptions, FolderPairInput, FolderPairResult};
    use crate::engine::db::SyncDb;
    use crate::engine::filter::Filter;
    use crate::engine::plan::{plan_pair, SyncConfig, SyncVariant};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::SystemTime;

    const BASE_TIME: u64 = 1_760_000_000;

    struct Fixture {
        tmp: tempfile::TempDir,
        left: PathBuf,
        right: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        Fixture { tmp, left, right }
    }

    fn write(base: &Path, rel: &str, content: &[u8], secs: i64) {
        let p = base.join(rel);
        fs::create_dir_all(p.parent().unwrap()).unwrap();
        fs::write(&p, content).unwrap();
        let t = SystemTime::UNIX_EPOCH + Duration::from_secs((BASE_TIME as i64 + secs) as u64);
        fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(t)
            .unwrap();
    }

    fn compare_fixture(f: &Fixture) -> FolderPairResult {
        let input = [FolderPairInput {
            left: f.left.display().to_string(),
            right: f.right.display().to_string(),
        }];
        compare(
            &input,
            &CompareOptions::default(),
            &Filter::none(),
            &CancelToken::new(),
            &|_| {},
        )
        .unwrap()
        .remove(0)
    }

    fn plan(f: &Fixture, variant: SyncVariant, db: Option<&SyncDb>) -> PairPlan {
        let cfg = SyncConfig {
            variant,
            ..Default::default()
        };
        plan_pair(&compare_fixture(f), &cfg, db)
    }

    fn permanent() -> ExecOptions {
        ExecOptions {
            deletion: DeletionPolicy::Permanent,
            verify_copies: false,
        }
    }

    fn run(p: &PairPlan, opts: &ExecOptions) -> Result<ExecReport, EngineError> {
        execute(
            p,
            opts,
            &CancelToken::new(),
            &PauseToken::new(),
            &winfs::free_space,
            &|_| {},
            &|f| panic!("unexpected failure: {f:?}"),
        )
    }

    fn assert_in_sync(f: &Fixture) {
        let r = compare_fixture(f);
        let bad: Vec<_> = r
            .pairs
            .iter()
            .filter(|p| p.category != Category::Equal)
            .map(|p| (&p.rel_path, p.category))
            .collect();
        assert!(bad.is_empty(), "not in sync: {bad:?}");
    }

    fn no_temp_files(dir: &Path) -> bool {
        let mut stack = vec![dir.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in fs::read_dir(&d).unwrap() {
                let e = e.unwrap();
                if e.file_type().unwrap().is_dir() {
                    stack.push(e.path());
                } else if e.file_name().to_string_lossy().ends_with(TEMP_SUFFIX) {
                    return false;
                }
            }
        }
        true
    }

    fn mixed(f: &Fixture) {
        write(&f.left, "equal.txt", b"same", 0);
        write(&f.right, "equal.txt", b"same", 0);
        write(&f.left, r"newdir\sub\new.txt", b"new", 0);
        write(&f.right, r"olddir\old.txt", b"old", 0);
        write(&f.left, "changed.txt", b"version 2", 100);
        write(&f.right, "changed.txt", b"version 1", 0);
        write(&f.left, "big.bin", &vec![42u8; 5_000_000], 0);
    }

    #[test]
    fn mirror_end_to_end() {
        let f = fixture();
        mixed(&f);
        let p = plan(&f, SyncVariant::Mirror, None);
        let report = run(&p, &permanent()).unwrap();
        assert_in_sync(&f);
        assert!(!f.right.join("olddir").exists());
        assert_eq!(fs::read(f.right.join("changed.txt")).unwrap(), b"version 2");
        // Modification time is preserved, so the next comparison sees "equal".
        assert_eq!(
            fs::metadata(f.right.join("big.bin")).unwrap().modified().unwrap(),
            fs::metadata(f.left.join("big.bin")).unwrap().modified().unwrap()
        );
        assert_eq!(report.done.create_right, 4); // newdir, newdir\sub, new.txt, big.bin
        assert_eq!(report.done.update_right, 1);
        assert_eq!(report.done.delete_right, 2);
        assert_eq!(report.bytes_copied, 3 + 9 + 5_000_000);
        assert!(report.failures.is_empty());
        assert!(no_temp_files(&f.right));
    }

    #[test]
    fn versioning_keeps_replaced_and_deleted_files() {
        let f = fixture();
        mixed(&f);
        let versions = f.tmp.path().join("versions");
        let opts = ExecOptions {
            deletion: DeletionPolicy::Versioning {
                folder: versions.display().to_string(),
            },
            verify_copies: true,
        };
        let p = plan(&f, SyncVariant::Mirror, None);
        let report = run(&p, &opts).unwrap();
        assert!(report.failures.is_empty());
        assert_in_sync(&f);
        let stamp_dir = fs::read_dir(&versions).unwrap().next().unwrap().unwrap().path();
        assert_eq!(fs::read(stamp_dir.join("changed.txt")).unwrap(), b"version 1");
        assert_eq!(fs::read(stamp_dir.join(r"olddir\old.txt")).unwrap(), b"old");
    }

    #[test]
    fn update_never_deletes() {
        let f = fixture();
        mixed(&f);
        let p = plan(&f, SyncVariant::Update, None);
        run(&p, &permanent()).unwrap();
        assert!(f.right.join(r"olddir\old.txt").exists());
        assert!(f.right.join(r"newdir\sub\new.txt").exists());
        assert_eq!(fs::read(f.right.join("changed.txt")).unwrap(), b"version 2");
    }

    #[test]
    fn two_way_end_to_end_with_database() {
        let f = fixture();
        mixed(&f);
        // First sync: copies both ways.
        let p = plan(&f, SyncVariant::TwoWay, None);
        run(&p, &permanent()).unwrap();
        assert_in_sync(&f);
        let db = SyncDb::from_comparison(&compare_fixture(&f));

        // Delete on the right, edit on the left, add on the right.
        fs::remove_file(f.right.join("equal.txt")).unwrap();
        write(&f.left, "changed.txt", b"version 3 from left", 200);
        write(&f.right, "from_right.txt", b"hello", 200);
        let p = plan(&f, SyncVariant::TwoWay, Some(&db));
        run(&p, &permanent()).unwrap();
        assert_in_sync(&f);
        assert!(!f.left.join("equal.txt").exists());
        assert_eq!(fs::read(f.right.join("changed.txt")).unwrap(), b"version 3 from left");
        assert!(f.left.join("from_right.txt").exists());
    }

    #[test]
    fn replaces_file_with_folder_and_back() {
        let f = fixture();
        write(&f.left, r"a\inside.txt", b"x", 0);
        write(&f.right, "a", b"file in the way", 0);
        write(&f.left, "b", b"now a file", 0);
        write(&f.right, r"b\old.txt", b"y", 0);
        run(&plan(&f, SyncVariant::Mirror, None), &permanent()).unwrap();
        assert_in_sync(&f);
        assert!(f.right.join("a").is_dir());
        assert!(f.right.join("b").is_file());
    }

    #[test]
    fn read_only_targets_are_handled() {
        let f = fixture();
        write(&f.left, "ro.txt", b"new", 100);
        write(&f.right, "ro.txt", b"old", 0);
        write(&f.right, "ro_delete.txt", b"old", 0);
        for name in ["ro.txt", "ro_delete.txt"] {
            let mut perms = fs::metadata(f.right.join(name)).unwrap().permissions();
            perms.set_readonly(true);
            fs::set_permissions(f.right.join(name), perms).unwrap();
        }
        run(&plan(&f, SyncVariant::Mirror, None), &permanent()).unwrap();
        assert_eq!(fs::read(f.right.join("ro.txt")).unwrap(), b"new");
        assert!(!f.right.join("ro_delete.txt").exists());
    }

    #[test]
    fn folder_with_excluded_content_is_kept() {
        let f = fixture();
        write(&f.right, r"old\keep.tmp", b"excluded", 0);
        write(&f.right, r"old\gone.txt", b"x", 0);
        write(&f.left, "a.txt", b"x", 0);
        let input = [FolderPairInput {
            left: f.left.display().to_string(),
            right: f.right.display().to_string(),
        }];
        let filter = Filter::new(&crate::engine::filter::FilterConfig {
            include: vec!["*".into()],
            exclude: vec!["*.tmp".into()],
        });
        let r = compare(
            &input,
            &CompareOptions::default(),
            &filter,
            &CancelToken::new(),
            &|_| {},
        )
        .unwrap()
        .remove(0);
        let p = plan_pair(&r, &SyncConfig::default(), None);
        let report = run(&p, &permanent()).unwrap();
        assert!(f.right.join(r"old\keep.tmp").exists());
        assert!(!f.right.join(r"old\gone.txt").exists());
        assert!(report.notes.iter().any(|n| n.contains("excluded")));
    }

    #[test]
    fn target_changed_after_compare_is_not_overwritten() {
        let f = fixture();
        write(&f.left, "a.txt", b"left", 100);
        write(&f.right, "a.txt", b"right", 0);
        let p = plan(&f, SyncVariant::Mirror, None);
        // Someone edits the backup copy between Compare and Synchronize.
        write(&f.right, "a.txt", b"edited meanwhile", 50);
        let failures = AtomicUsize::new(0);
        let report = execute(
            &p,
            &permanent(),
            &CancelToken::new(),
            &PauseToken::new(),
            &winfs::free_space,
            &|_| {},
            &|_| {
                failures.fetch_add(1, Ordering::SeqCst);
                ErrorDecision::Ignore
            },
        )
        .unwrap();
        assert_eq!(failures.load(Ordering::SeqCst), 1);
        assert_eq!(report.failures.len(), 1);
        assert_eq!(fs::read(f.right.join("a.txt")).unwrap(), b"edited meanwhile");
        assert!(no_temp_files(&f.right));
    }

    #[test]
    fn retry_after_fixing_the_problem() {
        let f = fixture();
        write(&f.left, "a.txt", b"x", 0);
        write(&f.left, "b.txt", b"y", 0);
        let p = plan(&f, SyncVariant::Mirror, None);
        let moved = f.tmp.path().join("a.txt.away");
        fs::rename(f.left.join("a.txt"), &moved).unwrap(); // source vanishes -> copy fails
        let calls = AtomicUsize::new(0);
        let report = execute(
            &p,
            &permanent(),
            &CancelToken::new(),
            &PauseToken::new(),
            &winfs::free_space,
            &|_| {},
            &|_| {
                calls.fetch_add(1, Ordering::SeqCst);
                fs::rename(&moved, f.left.join("a.txt")).unwrap(); // user fixes it
                ErrorDecision::Retry
            },
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(report.failures.is_empty());
        assert_in_sync(&f);
    }

    #[test]
    fn ignore_all_continues_with_the_rest() {
        let f = fixture();
        for i in 0..5 {
            write(&f.left, &format!("f{i}.txt"), b"x", 0);
        }
        let p = plan(&f, SyncVariant::Mirror, None);
        fs::remove_file(f.left.join("f1.txt")).unwrap();
        fs::remove_file(f.left.join("f3.txt")).unwrap();
        let calls = AtomicUsize::new(0);
        let report = execute(
            &p,
            &permanent(),
            &CancelToken::new(),
            &PauseToken::new(),
            &winfs::free_space,
            &|_| {},
            &|_| {
                calls.fetch_add(1, Ordering::SeqCst);
                ErrorDecision::IgnoreAll
            },
        )
        .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 1, "asked only once");
        assert_eq!(report.failures.len(), 2);
        assert_eq!(report.done.create_right, 3);
    }

    #[test]
    fn cancel_stops_cleanly() {
        let f = fixture();
        for i in 0..20 {
            write(&f.left, &format!("f{i:02}.bin"), &vec![1u8; 300_000], 0);
        }
        let p = plan(&f, SyncVariant::Mirror, None);
        let cancel = CancelToken::new();
        let report = execute(
            &p,
            &permanent(),
            &cancel,
            &PauseToken::new(),
            &winfs::free_space,
            &|prog| {
                if prog.items_done >= 3 {
                    cancel.cancel();
                }
            },
            &|f| panic!("{f:?}"),
        )
        .unwrap();
        assert!(report.cancelled);
        assert!(report.done.create_right < 20);
        assert!(no_temp_files(&f.right));
    }

    #[test]
    fn pause_and_resume() {
        let f = fixture();
        for i in 0..5 {
            write(&f.left, &format!("f{i}.txt"), b"x", 0);
        }
        let p = plan(&f, SyncVariant::Mirror, None);
        let pause = PauseToken::new();
        pause.set_paused(true);
        let resumer = {
            let pause = pause.clone();
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                pause.set_paused(false);
            })
        };
        let started = Instant::now();
        let report = execute(
            &p,
            &permanent(),
            &CancelToken::new(),
            &pause,
            &winfs::free_space,
            &|_| {},
            &|f| panic!("{f:?}"),
        )
        .unwrap();
        resumer.join().unwrap();
        assert!(started.elapsed() >= Duration::from_millis(250));
        assert_eq!(report.done.create_right, 5);
    }

    #[test]
    fn not_enough_space_stops_before_starting() {
        let f = fixture();
        write(&f.left, "a.bin", &[0u8; 1000], 0);
        let p = plan(&f, SyncVariant::Mirror, None);
        let err = execute(
            &p,
            &permanent(),
            &CancelToken::new(),
            &PauseToken::new(),
            &|_| Some(10),
            &|_| {},
            &|f| panic!("{f:?}"),
        )
        .unwrap_err();
        assert!(matches!(err, EngineError::NotEnoughSpace { needed: 1000, .. }));
        assert!(!f.right.join("a.bin").exists());
    }

    #[test]
    fn blocked_plan_is_refused() {
        let f = fixture();
        write(&f.right, "precious.txt", b"x", 0);
        let p = plan(&f, SyncVariant::Mirror, None); // left is empty -> blocked
        assert!(matches!(run(&p, &permanent()), Err(EngineError::Blocked(_))));
        assert!(f.right.join("precious.txt").exists());
    }

    #[test]
    fn leftover_temp_files_are_ignored_and_replaced() {
        let f = fixture();
        write(&f.left, "a.txt", b"real", 0);
        write(&f.right, &format!("a.txt{TEMP_SUFFIX}"), b"half-written junk", 0);
        let p = plan(&f, SyncVariant::Mirror, None);
        assert_eq!(p.items.len(), 1, "temp files are not part of the comparison");
        run(&p, &permanent()).unwrap();
        assert_eq!(fs::read(f.right.join("a.txt")).unwrap(), b"real");
        assert!(no_temp_files(&f.right));
    }

    #[test]
    fn auto_versions_path_layout() {
        assert_eq!(
            auto_versions_path(Path::new(r"F:\Backup\Docs\a.txt"), "2026-10-19 083512"),
            Some(PathBuf::from(r"F:\Relax-Versions\2026-10-19 083512\Backup\Docs\a.txt"))
        );
    }

    #[test]
    fn move_file_never_overwrites() {
        let tmp = tempfile::tempdir().unwrap();
        let a = tmp.path().join("a.txt");
        let b = tmp.path().join("b.txt");
        let target = tmp.path().join("v").join("x.txt");
        fs::write(&a, b"first").unwrap();
        fs::write(&b, b"second").unwrap();
        move_file(&a, &target).unwrap();
        move_file(&b, &target).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"first");
        assert_eq!(fs::read(tmp.path().join("v").join("x (2).txt")).unwrap(), b"second");
    }

    /// Moves a real file into this PC's Recycle Bin, so it is not run automatically.
    /// Run with: cargo test recycle_bin_for_real -- --ignored
    #[test]
    #[ignore]
    fn recycle_bin_for_real() {
        let f = fixture();
        write(&f.left, "a.txt", b"new", 100);
        write(&f.right, "a.txt", b"old - should be in the Recycle Bin", 0);
        let opts = ExecOptions::default();
        run(&plan(&f, SyncVariant::Mirror, None), &opts).unwrap();
        assert_eq!(fs::read(f.right.join("a.txt")).unwrap(), b"new");
    }
}
