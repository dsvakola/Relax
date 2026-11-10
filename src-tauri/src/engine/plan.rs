//! Sync planner: turns a comparison into a list of actions (create, update, delete) for each item.
//! Nothing is changed on disk here. The plan is what the user reviews before clicking Synchronize.
//!
//! Safety rules built into every plan:
//! - Nothing is deleted on one side while the folder on the other side is missing (drive unplugged).
//! - Nothing is deleted when the other side's folder is completely empty, unless the user allows it.
//! - Items inside a folder that could not be read are left untouched on both sides.
//! - Left and right must not be the same folder or inside each other.
//! - Folders are only deleted when everything inside them is deleted too.

use super::compare::{Category, FilePair, FolderPairInput, FolderPairResult};
use super::db::SyncDb;
use super::scan::{FsItem, ItemKind, TICKS_PER_SECOND};
use super::Side;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum SyncVariant {
    /// Changes on either side are copied to the other; deletions too (needs the sync database).
    TwoWay,
    /// Make the right side an exact copy of the left side.
    #[default]
    Mirror,
    /// Copy new and changed files to the right; never delete, never overwrite newer files.
    Update,
    /// User-defined direction for each category.
    Custom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Direction {
    ToRight,
    ToLeft,
    None,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DirectionTable {
    pub left_only: Direction,
    pub right_only: Direction,
    pub left_newer: Direction,
    pub right_newer: Direction,
    pub different: Direction,
    pub conflict: Direction,
}

impl DirectionTable {
    pub fn mirror() -> Self {
        Self {
            left_only: Direction::ToRight,
            right_only: Direction::ToRight,
            left_newer: Direction::ToRight,
            right_newer: Direction::ToRight,
            different: Direction::ToRight,
            conflict: Direction::ToRight,
        }
    }

    pub fn update() -> Self {
        Self {
            left_only: Direction::ToRight,
            right_only: Direction::None,
            left_newer: Direction::ToRight,
            right_newer: Direction::None,
            different: Direction::ToRight,
            conflict: Direction::None,
        }
    }

    fn get(&self, c: Category) -> Direction {
        match c {
            Category::Equal => Direction::None,
            Category::LeftOnly => self.left_only,
            Category::RightOnly => self.right_only,
            Category::LeftNewer => self.left_newer,
            Category::RightNewer => self.right_newer,
            Category::Different => self.different,
            Category::Conflict => self.conflict,
        }
    }
}

impl Default for DirectionTable {
    fn default() -> Self {
        Self::mirror()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SyncConfig {
    pub variant: SyncVariant,
    /// Used when `variant` is `Custom`.
    pub custom: DirectionTable,
    /// Allow deleting items because the other side's folder is completely empty.
    pub allow_sync_from_empty_folder: bool,
    /// Two-way: time differences up to this many seconds do not count as a change.
    pub time_tolerance_secs: u32,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            variant: SyncVariant::Mirror,
            custom: DirectionTable::mirror(),
            allow_sync_from_empty_folder: false,
            time_tolerance_secs: 2,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Action {
    None,
    /// Copy a new item to the right (or left) side.
    CreateRight,
    CreateLeft,
    /// Overwrite the item on the right (or left) side.
    UpdateRight,
    UpdateLeft,
    DeleteRight,
    DeleteLeft,
    /// Needs the user's decision; skipped until then.
    Conflict,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlannedItem {
    #[serde(flatten)]
    pub pair: FilePair,
    pub action: Action,
    /// Why this action was chosen, when it is not obvious.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SideStats {
    pub create_files: u64,
    pub create_folders: u64,
    pub update_files: u64,
    pub delete_files: u64,
    pub delete_folders: u64,
    /// Bytes that will be written to this side.
    pub bytes_to_write: u64,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanStats {
    pub left: SideStats,
    pub right: SideStats,
    pub conflicts: u64,
    pub unchanged: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairPlan {
    pub left: String,
    pub right: String,
    pub variant: SyncVariant,
    pub items: Vec<PlannedItem>,
    pub stats: PlanStats,
    /// Things the user should know before syncing (shown, but syncing is allowed).
    pub warnings: Vec<String>,
    /// If set, this folder pair must not be synced; the text explains why.
    pub blocked: Option<String>,
}

impl PairPlan {
    pub fn has_changes(&self) -> bool {
        self.items
            .iter()
            .any(|i| !matches!(i.action, Action::None | Action::Conflict))
    }
}

/// Builds the plan for one compared folder pair. `db` is the database from the last sync (Two-way only).
pub fn plan_pair(result: &FolderPairResult, cfg: &SyncConfig, db: Option<&SyncDb>) -> PairPlan {
    let items: Vec<PlannedItem> = result
        .pairs
        .iter()
        .map(|p| {
            let (action, note) = match cfg.variant {
                SyncVariant::TwoWay => two_way_action(p, db, cfg.time_tolerance_secs),
                SyncVariant::Mirror => (table_action(p, DirectionTable::mirror().get(p.category)), None),
                SyncVariant::Update => update_action(p),
                SyncVariant::Custom => (table_action(p, cfg.custom.get(p.category)), None),
            };
            PlannedItem {
                pair: p.clone(),
                action,
                note,
            }
        })
        .collect();
    finalize(result, cfg, items)
}

/// Applies the safety rules and computes statistics, warnings and the block reason.
/// Also used after the user changes actions by hand in the preview.
pub fn finalize(result: &FolderPairResult, cfg: &SyncConfig, mut items: Vec<PlannedItem>) -> PairPlan {
    protect_unreadable(result, &mut items);
    fix_folder_deletions(&mut items);

    let stats = compute_stats(&items);
    let mut warnings = Vec::new();
    let blocked = safety_block(result, cfg, &stats);

    for (side, s, total) in [
        ("left", &stats.left, result.stats.left_files + result.stats.left_folders),
        (
            "right",
            &stats.right,
            result.stats.right_files + result.stats.right_folders,
        ),
    ] {
        let deletes = s.delete_files + s.delete_folders;
        if deletes >= 10 && deletes * 2 > total {
            warnings.push(format!(
                "{deletes} of the {total} items on the {side} side will be deleted. Please check the preview carefully."
            ));
        }
    }
    if stats.conflicts > 0 {
        warnings.push(format!(
            "{} item(s) are conflicts and will be skipped until you choose what to do with them.",
            stats.conflicts
        ));
    }
    if !result.errors.is_empty() {
        warnings.push(format!(
            "{} item(s) could not be read. They and their counterparts on the other side will not be changed.",
            result.errors.len()
        ));
    }
    if !result.skipped_links.is_empty() {
        warnings.push(format!(
            "{} linked folder(s) or shortcut link(s) were skipped and will not be synchronized.",
            result.skipped_links.len()
        ));
    }

    PairPlan {
        left: result.left.clone(),
        right: result.right.clone(),
        variant: cfg.variant,
        items,
        stats,
        warnings,
        blocked,
    }
}

/// Maps a direction to an action, depending on which side currently has the item.
pub fn table_action(p: &FilePair, dir: Direction) -> Action {
    if p.category == Category::Equal {
        return Action::None;
    }
    let (l, r) = (p.left.is_some(), p.right.is_some());
    match dir {
        Direction::ToRight => match (l, r) {
            (true, false) => Action::CreateRight,
            (false, true) => Action::DeleteRight,
            _ => Action::UpdateRight,
        },
        Direction::ToLeft => match (l, r) {
            (false, true) => Action::CreateLeft,
            (true, false) => Action::DeleteLeft,
            _ => Action::UpdateLeft,
        },
        Direction::None if p.category == Category::Conflict => Action::Conflict,
        Direction::None => Action::None,
    }
}

fn update_action(p: &FilePair) -> (Action, Option<String>) {
    let action = table_action(p, DirectionTable::update().get(p.category));
    let note = match p.category {
        Category::RightNewer => Some("The right file is newer. Update mode never overwrites newer files.".into()),
        Category::RightOnly => Some("Only on the right. Update mode never deletes.".into()),
        _ => None,
    };
    (action, note)
}

fn changed_since(item: &FsItem, db_kind: ItemKind, db_size: u64, db_modified: i64, tol_ticks: i64) -> bool {
    if item.kind != db_kind {
        return true;
    }
    item.kind == ItemKind::File && (item.size != db_size || (item.modified - db_modified).abs() > tol_ticks)
}

fn two_way_action(p: &FilePair, db: Option<&SyncDb>, tol_secs: u32) -> (Action, Option<String>) {
    if p.category == Category::Equal {
        return (Action::None, None);
    }
    let tol = tol_secs as i64 * TICKS_PER_SECOND;
    let entry = db.and_then(|d| d.get(&p.rel_path));

    match (&p.left, &p.right, entry) {
        // Never synced before: new on one side -> copy it; differences -> newer wins.
        (_, _, None) => {
            let action = match p.category {
                Category::LeftOnly => Action::CreateRight,
                Category::RightOnly => Action::CreateLeft,
                Category::LeftNewer => Action::UpdateRight,
                Category::RightNewer => Action::UpdateLeft,
                _ => Action::Conflict,
            };
            (action, None)
        }
        (Some(l), None, Some(e)) => {
            if changed_since(l, e.kind, e.size, e.left_modified, tol) {
                (
                    Action::Conflict,
                    Some("Deleted on the right, but changed on the left since the last sync.".into()),
                )
            } else {
                (
                    Action::DeleteLeft,
                    Some("Deleted on the right since the last sync.".into()),
                )
            }
        }
        (None, Some(r), Some(e)) => {
            if changed_since(r, e.kind, e.size, e.right_modified, tol) {
                (
                    Action::Conflict,
                    Some("Deleted on the left, but changed on the right since the last sync.".into()),
                )
            } else {
                (
                    Action::DeleteRight,
                    Some("Deleted on the left since the last sync.".into()),
                )
            }
        }
        (Some(l), Some(r), Some(e)) => {
            if l.kind != r.kind {
                return (Action::Conflict, None);
            }
            let lc = changed_since(l, e.kind, e.size, e.left_modified, tol);
            let rc = changed_since(r, e.kind, e.size, e.right_modified, tol);
            match (lc, rc) {
                (true, false) => (
                    Action::UpdateRight,
                    Some("Changed on the left since the last sync.".into()),
                ),
                (false, true) => (
                    Action::UpdateLeft,
                    Some("Changed on the right since the last sync.".into()),
                ),
                (true, true) => (
                    Action::Conflict,
                    Some("Changed on both sides since the last sync.".into()),
                ),
                (false, false) => (Action::Conflict, None),
            }
        }
        (None, None, Some(_)) => (Action::None, None),
    }
}

/// Items in or below a folder (or file) that could not be read must not be touched on either side;
/// otherwise their counterparts would look "deleted" and be removed.
fn protect_unreadable(result: &FolderPairResult, items: &mut [PlannedItem]) {
    let mut blocked_rel: Vec<String> = Vec::new();
    for e in &result.errors {
        let base = match e.side {
            Side::Left => &result.left,
            Side::Right => &result.right,
        };
        let base = normalize(base);
        let path = normalize(&e.path);
        if path == base {
            blocked_rel.push(String::new());
        } else if let Some(rel) = path.strip_prefix(&format!("{base}\\")) {
            blocked_rel.push(rel.to_string());
        }
    }
    if blocked_rel.is_empty() {
        return;
    }
    for item in items.iter_mut() {
        let rel = item.pair.rel_path.to_lowercase();
        let inside = blocked_rel
            .iter()
            .any(|b| b.is_empty() || rel == *b || rel.starts_with(&format!("{b}\\")));
        if inside && item.action != Action::None {
            item.action = Action::None;
            item.note = Some("Skipped because a matching item could not be read.".into());
        }
    }
}

/// A folder may only be deleted if everything inside it is deleted as well. Otherwise it is kept,
/// or created on the other side if new items are being copied into it there.
fn fix_folder_deletions(items: &mut [PlannedItem]) {
    for i in 0..items.len() {
        let del = items[i].action;
        if !(items[i].pair.left.as_ref().or(items[i].pair.right.as_ref())).is_some_and(|x| x.kind == ItemKind::Folder)
            || !matches!(del, Action::DeleteLeft | Action::DeleteRight)
        {
            continue;
        }
        let prefix = format!("{}\\", items[i].pair.rel_path.to_lowercase());
        let create_other = if del == Action::DeleteLeft {
            Action::CreateRight
        } else {
            Action::CreateLeft
        };
        let mut all_deleted = true;
        let mut creates_other = false;
        for child in items[i + 1..].iter() {
            if !child.pair.rel_path.to_lowercase().starts_with(&prefix) {
                break;
            }
            if child.action != del {
                all_deleted = false;
            }
            if child.action == create_other {
                creates_other = true;
            }
        }
        if !all_deleted {
            items[i].action = if creates_other { create_other } else { Action::None };
            items[i].note = Some("Kept, because it contains items that are not being deleted.".into());
        }
    }
}

fn compute_stats(items: &[PlannedItem]) -> PlanStats {
    let mut s = PlanStats::default();
    for it in items {
        let is_folder = it
            .pair
            .left
            .as_ref()
            .or(it.pair.right.as_ref())
            .is_some_and(|x| x.kind == ItemKind::Folder);
        let left_size = it.pair.left.as_ref().map_or(0, |x| x.size);
        let right_size = it.pair.right.as_ref().map_or(0, |x| x.size);
        match it.action {
            Action::None => s.unchanged += 1,
            Action::Conflict => s.conflicts += 1,
            Action::CreateRight if is_folder => s.right.create_folders += 1,
            Action::CreateLeft if is_folder => s.left.create_folders += 1,
            Action::CreateRight => {
                s.right.create_files += 1;
                s.right.bytes_to_write += left_size;
            }
            Action::CreateLeft => {
                s.left.create_files += 1;
                s.left.bytes_to_write += right_size;
            }
            Action::UpdateRight => {
                s.right.update_files += 1;
                s.right.bytes_to_write += left_size;
            }
            Action::UpdateLeft => {
                s.left.update_files += 1;
                s.left.bytes_to_write += right_size;
            }
            Action::DeleteRight if is_folder => s.right.delete_folders += 1,
            Action::DeleteLeft if is_folder => s.left.delete_folders += 1,
            Action::DeleteRight => s.right.delete_files += 1,
            Action::DeleteLeft => s.left.delete_files += 1,
        }
    }
    s
}

fn safety_block(result: &FolderPairResult, cfg: &SyncConfig, stats: &PlanStats) -> Option<String> {
    if let Some(msg) = folder_overlap(&result.left, &result.right) {
        return Some(msg);
    }
    let deletes_left = stats.left.delete_files + stats.left.delete_folders;
    let deletes_right = stats.right.delete_files + stats.right.delete_folders;
    if !result.left_exists && deletes_right > 0 {
        return Some(format!(
            "The left folder was not found:\n{}\nRelax will not delete anything on the right while it is missing. Check that the drive is connected.",
            result.left
        ));
    }
    if !result.right_exists && deletes_left > 0 {
        return Some(format!(
            "The right folder was not found:\n{}\nRelax will not delete anything on the left while it is missing. Check that the drive is connected.",
            result.right
        ));
    }
    if !cfg.allow_sync_from_empty_folder {
        let left_empty = result.left_exists && result.stats.left_files + result.stats.left_folders == 0;
        let right_empty = result.right_exists && result.stats.right_files + result.stats.right_folders == 0;
        if left_empty && deletes_right > 0 {
            return Some(format!(
                "The left folder is empty:\n{}\nSyncing would delete {deletes_right} item(s) on the right. If this is really what you want, allow syncing from an empty folder in the settings.",
                result.left
            ));
        }
        if right_empty && deletes_left > 0 {
            return Some(format!(
                "The right folder is empty:\n{}\nSyncing would delete {deletes_left} item(s) on the left. If this is really what you want, allow syncing from an empty folder in the settings.",
                result.right
            ));
        }
    }
    None
}

fn normalize(path: &str) -> String {
    path.trim().replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

/// Returns an explanation if the two folders are the same or one is inside the other.
pub fn folder_overlap(left: &str, right: &str) -> Option<String> {
    let (l, r) = (normalize(left), normalize(right));
    if l == r {
        Some(format!("Left and right are the same folder:\n{left}"))
    } else if r.starts_with(&format!("{l}\\")) {
        Some(format!(
            "The right folder is inside the left folder. This would copy the backup into itself again and again.\nLeft: {left}\nRight: {right}"
        ))
    } else if l.starts_with(&format!("{r}\\")) {
        Some(format!(
            "The left folder is inside the right folder.\nLeft: {left}\nRight: {right}"
        ))
    } else {
        None
    }
}

/// Warnings about folder pairs that write into the same or overlapping folders.
pub fn check_folder_pairs(pairs: &[FolderPairInput]) -> Vec<String> {
    let mut warnings = Vec::new();
    for (i, a) in pairs.iter().enumerate() {
        for b in &pairs[i + 1..] {
            for (x, y) in [(&a.right, &b.right), (&a.left, &b.right), (&a.right, &b.left)] {
                if folder_overlap(x, y).is_some() {
                    warnings.push(format!(
                        "Two folder pairs use the same or overlapping folders:\n{x}\n{y}\nThey may change each other's files."
                    ));
                }
            }
        }
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::compare::{compare, CompareOptions, CompareStats};
    use crate::engine::filter::Filter;
    use crate::engine::{CancelToken, ItemError};
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime};

    const BASE_TIME: u64 = 1_760_000_000;

    struct Fixture {
        _tmp: tempfile::TempDir,
        left: PathBuf,
        right: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        let left = tmp.path().join("left");
        let right = tmp.path().join("right");
        fs::create_dir_all(&left).unwrap();
        fs::create_dir_all(&right).unwrap();
        Fixture { _tmp: tmp, left, right }
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

    fn action(p: &PairPlan, rel: &str) -> Action {
        p.items
            .iter()
            .find(|i| i.pair.rel_path.eq_ignore_ascii_case(rel))
            .unwrap_or_else(|| panic!("{rel} missing"))
            .action
    }

    /// Left and right with one example of every category.
    fn mixed_fixture() -> Fixture {
        let f = fixture();
        write(&f.left, "equal.txt", b"same", 0);
        write(&f.right, "equal.txt", b"same", 0);
        write(&f.left, r"newdir\new.txt", b"new", 0);
        write(&f.right, "old.txt", b"old", 0);
        write(&f.left, "left_newer.txt", b"v2", 100);
        write(&f.right, "left_newer.txt", b"v1", 0);
        write(&f.left, "right_newer.txt", b"v1", 0);
        write(&f.right, "right_newer.txt", b"v2", 100);
        write(&f.left, "conflict.txt", b"short", 0);
        write(&f.right, "conflict.txt", b"much longer", 0);
        f
    }

    #[test]
    fn mirror_makes_right_identical_to_left() {
        let f = mixed_fixture();
        let p = plan(&f, SyncVariant::Mirror, None);
        assert_eq!(action(&p, "equal.txt"), Action::None);
        assert_eq!(action(&p, "newdir"), Action::CreateRight);
        assert_eq!(action(&p, r"newdir\new.txt"), Action::CreateRight);
        assert_eq!(action(&p, "old.txt"), Action::DeleteRight);
        assert_eq!(action(&p, "left_newer.txt"), Action::UpdateRight);
        assert_eq!(action(&p, "right_newer.txt"), Action::UpdateRight);
        assert_eq!(action(&p, "conflict.txt"), Action::UpdateRight);
        assert!(p.blocked.is_none());
        assert_eq!(p.stats.right.create_files, 1);
        assert_eq!(p.stats.right.create_folders, 1);
        assert_eq!(p.stats.right.update_files, 3);
        assert_eq!(p.stats.right.delete_files, 1);
        assert_eq!(p.stats.right.bytes_to_write, 3 + 2 + 2 + 5);
        assert_eq!(p.stats.left, SideStats::default());
    }

    #[test]
    fn update_never_deletes_or_overwrites_newer() {
        let f = mixed_fixture();
        let p = plan(&f, SyncVariant::Update, None);
        assert_eq!(action(&p, r"newdir\new.txt"), Action::CreateRight);
        assert_eq!(action(&p, "old.txt"), Action::None);
        assert_eq!(action(&p, "left_newer.txt"), Action::UpdateRight);
        assert_eq!(action(&p, "right_newer.txt"), Action::None);
        assert_eq!(action(&p, "conflict.txt"), Action::Conflict);
        assert_eq!(p.stats.right.delete_files + p.stats.left.delete_files, 0);
        assert_eq!(p.stats.left, SideStats::default());
    }

    #[test]
    fn two_way_first_sync_copies_both_ways() {
        let f = mixed_fixture();
        let p = plan(&f, SyncVariant::TwoWay, None);
        assert_eq!(action(&p, r"newdir\new.txt"), Action::CreateRight);
        assert_eq!(action(&p, "old.txt"), Action::CreateLeft);
        assert_eq!(action(&p, "left_newer.txt"), Action::UpdateRight);
        assert_eq!(action(&p, "right_newer.txt"), Action::UpdateLeft);
        assert_eq!(action(&p, "conflict.txt"), Action::Conflict);
        assert_eq!(p.stats.left.delete_files + p.stats.right.delete_files, 0);
    }

    /// Two folders that were just synced, plus the database recorded at that moment.
    fn synced_fixture() -> (Fixture, SyncDb) {
        let f = fixture();
        for base in [&f.left, &f.right] {
            write(base, "keep.txt", b"keep", 0);
            write(base, "del_on_right.txt", b"x", 0);
            write(base, "del_on_left.txt", b"x", 0);
            write(base, "edit_left.txt", b"v1", 0);
            write(base, "edit_right.txt", b"v1", 0);
            write(base, "edit_both.txt", b"v1", 0);
            write(base, "del_right_edit_left.txt", b"v1", 0);
            write(base, r"gone_dir\a.txt", b"a", 0);
            write(base, r"gone_dir_but_new_file\a.txt", b"a", 0);
        }
        let db = SyncDb::from_comparison(&compare_fixture(&f));
        (f, db)
    }

    #[test]
    fn two_way_uses_database_to_detect_deletions_and_edits() {
        let (f, db) = synced_fixture();
        fs::remove_file(f.right.join("del_on_right.txt")).unwrap();
        fs::remove_file(f.left.join("del_on_left.txt")).unwrap();
        write(&f.left, "edit_left.txt", b"v2 left", 50);
        write(&f.right, "edit_right.txt", b"v2 right", 50);
        write(&f.left, "edit_both.txt", b"left edit", 50);
        write(&f.right, "edit_both.txt", b"right edit!", 60);
        fs::remove_file(f.right.join("del_right_edit_left.txt")).unwrap();
        write(&f.left, "del_right_edit_left.txt", b"v2", 50);
        fs::remove_dir_all(f.right.join("gone_dir")).unwrap();
        fs::remove_dir_all(f.right.join("gone_dir_but_new_file")).unwrap();
        write(&f.left, r"gone_dir_but_new_file\brand_new.txt", b"new", 50);

        let p = plan(&f, SyncVariant::TwoWay, Some(&db));
        assert_eq!(action(&p, "keep.txt"), Action::None);
        assert_eq!(action(&p, "del_on_right.txt"), Action::DeleteLeft);
        assert_eq!(action(&p, "del_on_left.txt"), Action::DeleteRight);
        assert_eq!(action(&p, "edit_left.txt"), Action::UpdateRight);
        assert_eq!(action(&p, "edit_right.txt"), Action::UpdateLeft);
        assert_eq!(action(&p, "edit_both.txt"), Action::Conflict);
        assert_eq!(action(&p, "del_right_edit_left.txt"), Action::Conflict);
        assert_eq!(action(&p, "gone_dir"), Action::DeleteLeft);
        assert_eq!(action(&p, r"gone_dir\a.txt"), Action::DeleteLeft);
        // The folder was deleted on the right, but a new file appeared in it on the left:
        // the folder is re-created on the right instead of being deleted.
        assert_eq!(action(&p, "gone_dir_but_new_file"), Action::CreateRight);
        assert_eq!(action(&p, r"gone_dir_but_new_file\a.txt"), Action::DeleteLeft);
        assert_eq!(action(&p, r"gone_dir_but_new_file\brand_new.txt"), Action::CreateRight);
    }

    #[test]
    fn custom_directions() {
        let f = mixed_fixture();
        let cfg = SyncConfig {
            variant: SyncVariant::Custom,
            custom: DirectionTable {
                left_only: Direction::ToLeft, // i.e. delete items only on the left
                right_only: Direction::ToLeft,
                left_newer: Direction::None,
                right_newer: Direction::ToLeft,
                different: Direction::None,
                conflict: Direction::None,
            },
            ..Default::default()
        };
        let p = plan_pair(&compare_fixture(&f), &cfg, None);
        assert_eq!(action(&p, "newdir"), Action::DeleteLeft);
        assert_eq!(action(&p, r"newdir\new.txt"), Action::DeleteLeft);
        assert_eq!(action(&p, "old.txt"), Action::CreateLeft);
        assert_eq!(action(&p, "left_newer.txt"), Action::None);
        assert_eq!(action(&p, "right_newer.txt"), Action::UpdateLeft);
        assert_eq!(action(&p, "conflict.txt"), Action::Conflict);
    }

    #[test]
    fn missing_source_blocks_all_deletion() {
        let f = fixture();
        write(&f.right, "precious.txt", b"x", 0);
        let input = [FolderPairInput {
            left: f.left.join("usb-not-plugged-in").display().to_string(),
            right: f.right.display().to_string(),
        }];
        let r = compare(
            &input,
            &CompareOptions::default(),
            &Filter::none(),
            &CancelToken::new(),
            &|_| {},
        )
        .unwrap()
        .remove(0);
        let p = plan_pair(&r, &SyncConfig::default(), None);
        assert!(p.blocked.as_deref().unwrap().contains("not found"));
    }

    #[test]
    fn empty_source_blocks_mirror_unless_allowed() {
        let f = fixture();
        write(&f.right, "precious.txt", b"x", 0);
        let r = compare_fixture(&f);
        let p = plan_pair(&r, &SyncConfig::default(), None);
        assert!(p.blocked.as_deref().unwrap().contains("empty"));
        let cfg = SyncConfig {
            allow_sync_from_empty_folder: true,
            ..Default::default()
        };
        assert!(plan_pair(&r, &cfg, None).blocked.is_none());
    }

    #[test]
    fn first_backup_to_empty_target_is_allowed() {
        let f = fixture();
        write(&f.left, "a.txt", b"x", 0);
        let p = plan(&f, SyncVariant::Mirror, None);
        assert!(p.blocked.is_none());
        assert_eq!(action(&p, "a.txt"), Action::CreateRight);
    }

    #[test]
    fn nested_or_same_folders_are_blocked() {
        assert!(folder_overlap(r"D:\Data", r"d:\data\").is_some());
        assert!(folder_overlap(r"D:\Data", r"D:\Data\Backup").is_some());
        assert!(folder_overlap(r"D:\Data\Backup", r"D:\Data").is_some());
        assert!(folder_overlap(r"D:\Data", r"D:\Data2").is_none());
        assert!(folder_overlap(r"D:\", r"E:\").is_none());

        let f = fixture();
        write(&f.left, "a.txt", b"x", 0);
        let input = [FolderPairInput {
            left: f.left.display().to_string(),
            right: f.left.join("backup").display().to_string(),
        }];
        let r = compare(
            &input,
            &CompareOptions::default(),
            &Filter::none(),
            &CancelToken::new(),
            &|_| {},
        )
        .unwrap()
        .remove(0);
        assert!(plan_pair(&r, &SyncConfig::default(), None).blocked.is_some());
    }

    #[test]
    fn overlapping_folder_pairs_warn() {
        let pairs = [
            FolderPairInput {
                left: r"D:\A".into(),
                right: r"F:\Backup".into(),
            },
            FolderPairInput {
                left: r"D:\B".into(),
                right: r"F:\Backup\B".into(),
            },
        ];
        assert_eq!(check_folder_pairs(&pairs).len(), 1);
    }

    #[test]
    fn mass_deletion_warns() {
        let f = fixture();
        write(&f.left, "keep.txt", b"x", 0);
        write(&f.right, "keep.txt", b"x", 0);
        for i in 0..20 {
            write(&f.right, &format!("extra{i}.txt"), b"x", 0);
        }
        let p = plan(&f, SyncVariant::Mirror, None);
        assert!(p.blocked.is_none());
        assert!(
            p.warnings.iter().any(|w| w.contains("20 of the 21")),
            "{:?}",
            p.warnings
        );
    }

    #[test]
    fn unreadable_folder_protects_its_counterpart() {
        // Simulates a left sub-folder that could not be read: its files are missing from the scan,
        // so a naive mirror would delete them on the right.
        let item = |rel: &str| FsItem {
            rel_path: rel.into(),
            kind: if rel.ends_with(".txt") {
                ItemKind::File
            } else {
                ItemKind::Folder
            },
            size: 1,
            modified: 0,
        };
        let right_only = |rel: &str| FilePair {
            rel_path: rel.into(),
            left: None,
            right: Some(item(rel)),
            category: Category::RightOnly,
            conflict_reason: None,
        };
        let both = |rel: &str| FilePair {
            rel_path: rel.into(),
            left: Some(item(rel)),
            right: Some(item(rel)),
            category: Category::Equal,
            conflict_reason: None,
        };
        let result = FolderPairResult {
            left: r"D:\Data".into(),
            right: r"F:\Backup".into(),
            left_exists: true,
            right_exists: true,
            pairs: vec![
                both("Locked"),
                right_only(r"Locked\secret.txt"),
                right_only("really_deleted.txt"),
            ],
            stats: CompareStats {
                left_folders: 1,
                right_folders: 1,
                right_files: 2,
                ..Default::default()
            },
            errors: vec![ItemError {
                side: Side::Left,
                path: r"D:\Data\Locked".into(),
                message: "Access is denied.".into(),
            }],
            skipped_links: vec![],
        };
        let p = plan_pair(&result, &SyncConfig::default(), None);
        assert_eq!(action(&p, r"Locked\secret.txt"), Action::None);
        assert_eq!(action(&p, "really_deleted.txt"), Action::DeleteRight);
        assert!(p.warnings.iter().any(|w| w.contains("could not be read")));
    }

    #[test]
    fn mirror_replaces_file_with_folder() {
        let f = fixture();
        write(&f.left, r"thing\inside.txt", b"x", 0);
        write(&f.right, "thing", b"old file", 0);
        let p = plan(&f, SyncVariant::Mirror, None);
        assert_eq!(action(&p, "thing"), Action::UpdateRight);
        assert_eq!(action(&p, r"thing\inside.txt"), Action::CreateRight);
    }
}
