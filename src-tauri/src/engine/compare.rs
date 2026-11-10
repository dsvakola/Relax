//! Compares the left and right folder of each folder pair and puts every item into a category.
//! Nothing is changed on disk here; this is what the preview grid shows.

use super::filter::Filter;
use super::scan::{scan_folder, FsItem, ItemKind, TICKS_PER_SECOND};
use super::{CancelToken, EngineError, ItemError, Side};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs::File;
use std::io::{ErrorKind, Read};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub enum CompareVariant {
    /// Same size and same modification time (within tolerance) means equal. Fast; the default.
    #[default]
    TimeAndSize,
    /// Byte-by-byte comparison of files with the same size. Slow but certain.
    Content,
    /// Same size means equal.
    Size,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CompareOptions {
    pub variant: CompareVariant,
    /// Time differences up to this many seconds count as equal. 2 s covers FAT32/exFAT USB drives.
    pub time_tolerance_secs: u32,
    /// Treat differences of exactly one hour as equal (daylight-saving shifts on FAT drives).
    pub ignore_hour_shift: bool,
}

impl Default for CompareOptions {
    fn default() -> Self {
        Self {
            variant: CompareVariant::TimeAndSize,
            time_tolerance_secs: 2,
            ignore_hour_shift: false,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Category {
    Equal,
    LeftOnly,
    RightOnly,
    LeftNewer,
    RightNewer,
    /// Contents or sizes differ and time does not tell which is newer.
    Different,
    /// Needs a human decision (see `conflict_reason`).
    Conflict,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FilePair {
    /// Relative path as spelled on the left (or right, if only there).
    pub rel_path: String,
    pub left: Option<FsItem>,
    pub right: Option<FsItem>,
    pub category: Category,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conflict_reason: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CompareStats {
    pub equal: u64,
    pub left_only: u64,
    pub right_only: u64,
    pub left_newer: u64,
    pub right_newer: u64,
    pub different: u64,
    pub conflict: u64,
    pub left_files: u64,
    pub left_folders: u64,
    pub left_bytes: u64,
    pub right_files: u64,
    pub right_folders: u64,
    pub right_bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPairInput {
    pub left: String,
    pub right: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderPairResult {
    pub left: String,
    pub right: String,
    /// False when the folder does not exist (e.g. a backup folder not created yet, or a drive not plugged in).
    pub left_exists: bool,
    pub right_exists: bool,
    pub pairs: Vec<FilePair>,
    pub stats: CompareStats,
    pub errors: Vec<ItemError>,
    pub skipped_links: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "phase")]
pub enum CompareProgress {
    Scanning {
        pair_index: usize,
        items_found: u64,
    },
    ComparingContent {
        pair_index: usize,
        bytes_done: u64,
        bytes_total: u64,
    },
}

/// Compares every folder pair. Stops only on cancel or when a pair cannot be compared at all.
pub fn compare(
    folder_pairs: &[FolderPairInput],
    options: &CompareOptions,
    filter: &Filter,
    cancel: &CancelToken,
    on_progress: &(dyn Fn(CompareProgress) + Sync),
) -> Result<Vec<FolderPairResult>, EngineError> {
    folder_pairs
        .iter()
        .enumerate()
        .map(|(i, fp)| compare_pair(i, fp, options, filter, cancel, on_progress))
        .collect()
}

fn compare_pair(
    pair_index: usize,
    fp: &FolderPairInput,
    options: &CompareOptions,
    filter: &Filter,
    cancel: &CancelToken,
    on_progress: &(dyn Fn(CompareProgress) + Sync),
) -> Result<FolderPairResult, EngineError> {
    let left_base = PathBuf::from(fp.left.trim());
    let right_base = PathBuf::from(fp.right.trim());

    // Scan both sides at the same time; they are usually on different drives.
    let left_count = std::sync::atomic::AtomicU64::new(0);
    let right_count = std::sync::atomic::AtomicU64::new(0);
    let report = || {
        on_progress(CompareProgress::Scanning {
            pair_index,
            items_found: left_count.load(std::sync::atomic::Ordering::Relaxed)
                + right_count.load(std::sync::atomic::Ordering::Relaxed),
        })
    };
    let (left_scan, right_scan) = std::thread::scope(|s| {
        let l = s.spawn(|| {
            scan_folder(&left_base, Side::Left, filter, cancel, &|n| {
                left_count.store(n, std::sync::atomic::Ordering::Relaxed);
                report();
            })
        });
        let r = scan_folder(&right_base, Side::Right, filter, cancel, &|n| {
            right_count.store(n, std::sync::atomic::Ordering::Relaxed);
            report();
        });
        (l.join().unwrap_or(Err(EngineError::Cancelled)), r)
    });
    let left_scan = left_scan?;
    let right_scan = right_scan?;

    if left_scan.is_none() && right_scan.is_none() {
        return Err(EngineError::BothFoldersMissing {
            left: fp.left.clone(),
            right: fp.right.clone(),
        });
    }
    let left_exists = left_scan.is_some();
    let right_exists = right_scan.is_some();
    let left_scan = left_scan.unwrap_or_default();
    let right_scan = right_scan.unwrap_or_default();

    let mut stats = CompareStats {
        left_files: left_scan.files,
        left_folders: left_scan.folders,
        left_bytes: left_scan.bytes,
        right_files: right_scan.files,
        right_folders: right_scan.folders,
        right_bytes: right_scan.bytes,
        ..Default::default()
    };
    let mut errors = left_scan.errors;
    errors.extend(right_scan.errors);
    let mut skipped_links = left_scan.skipped_links;
    skipped_links.extend(right_scan.skipped_links);

    let mut pairs = merge(left_scan.items, right_scan.items, options);

    if options.variant == CompareVariant::Content {
        compare_contents(pair_index, &left_base, &right_base, &mut pairs, cancel, on_progress)?;
    }

    for p in &pairs {
        match p.category {
            Category::Equal => stats.equal += 1,
            Category::LeftOnly => stats.left_only += 1,
            Category::RightOnly => stats.right_only += 1,
            Category::LeftNewer => stats.left_newer += 1,
            Category::RightNewer => stats.right_newer += 1,
            Category::Different => stats.different += 1,
            Category::Conflict => stats.conflict += 1,
        }
    }

    Ok(FolderPairResult {
        left: fp.left.clone(),
        right: fp.right.clone(),
        left_exists,
        right_exists,
        pairs,
        stats,
        errors,
        skipped_links,
    })
}

/// Joins both sides by case-insensitive path and categorizes each pair (without reading file contents).
fn merge(
    mut left: BTreeMap<String, FsItem>,
    mut right: BTreeMap<String, FsItem>,
    options: &CompareOptions,
) -> Vec<FilePair> {
    let mut keys: Vec<String> = left.keys().chain(right.keys()).cloned().collect();
    keys.sort();
    keys.dedup();

    keys.into_iter()
        .map(|key| {
            let l = left.remove(&key);
            let r = right.remove(&key);
            let rel_path = l
                .as_ref()
                .or(r.as_ref())
                .map(|i| i.rel_path.clone())
                .unwrap_or_default();
            let (category, conflict_reason) = categorize(l.as_ref(), r.as_ref(), options);
            FilePair {
                rel_path,
                left: l,
                right: r,
                category,
                conflict_reason,
            }
        })
        .collect()
}

fn categorize(l: Option<&FsItem>, r: Option<&FsItem>, options: &CompareOptions) -> (Category, Option<String>) {
    let (l, r) = match (l, r) {
        (Some(l), Some(r)) => (l, r),
        (Some(_), None) => return (Category::LeftOnly, None),
        (None, Some(_)) => return (Category::RightOnly, None),
        (None, None) => unreachable!("merge only produces keys present on at least one side"),
    };

    match (l.kind, r.kind) {
        (ItemKind::Folder, ItemKind::Folder) => return (Category::Equal, None),
        (ItemKind::File, ItemKind::File) => {}
        _ => {
            return (
                Category::Conflict,
                Some("This name is a file on one side and a folder on the other.".into()),
            )
        }
    }

    match options.variant {
        CompareVariant::Size | CompareVariant::Content => {
            // Content mode re-checks equal-sized files later by reading them.
            if l.size == r.size {
                (Category::Equal, None)
            } else {
                (Category::Different, None)
            }
        }
        CompareVariant::TimeAndSize => {
            let tol = options.time_tolerance_secs as i64 * TICKS_PER_SECOND;
            let diff = l.modified - r.modified;
            let hour = 3600 * TICKS_PER_SECOND;
            let same_time = diff.abs() <= tol || (options.ignore_hour_shift && (diff.abs() - hour).abs() <= tol);
            if same_time {
                if l.size == r.size {
                    (Category::Equal, None)
                } else {
                    (
                        Category::Conflict,
                        Some("Both files have the same date but different sizes.".into()),
                    )
                }
            } else if diff > 0 {
                (Category::LeftNewer, None)
            } else {
                (Category::RightNewer, None)
            }
        }
    }
}

/// Reads every equal-sized file pair and downgrades it to `Different` if any byte differs.
fn compare_contents(
    pair_index: usize,
    left_base: &Path,
    right_base: &Path,
    pairs: &mut [FilePair],
    cancel: &CancelToken,
    on_progress: &(dyn Fn(CompareProgress) + Sync),
) -> Result<(), EngineError> {
    let is_candidate = |p: &FilePair| {
        p.category == Category::Equal && matches!((&p.left, &p.right), (Some(l), Some(_)) if l.kind == ItemKind::File)
    };
    let bytes_total: u64 = pairs
        .iter()
        .filter(|p| is_candidate(p))
        .map(|p| p.left.as_ref().map_or(0, |l| l.size))
        .sum();
    let mut bytes_done = 0u64;

    for p in pairs.iter_mut() {
        if !is_candidate(p) {
            continue;
        }
        let (Some(l), Some(r)) = (&p.left, &p.right) else {
            continue;
        };
        let lp = left_base.join(&l.rel_path);
        let rp = right_base.join(&r.rel_path);
        let mut tick = |n: u64| {
            bytes_done += n;
            on_progress(CompareProgress::ComparingContent {
                pair_index,
                bytes_done,
                bytes_total,
            });
        };
        match files_identical(&lp, &rp, cancel, &mut tick) {
            Ok(true) => {}
            Ok(false) => p.category = Category::Different,
            Err(e) if e.kind() == ErrorKind::Interrupted && cancel.is_cancelled() => {
                return Err(EngineError::Cancelled)
            }
            Err(e) => {
                p.category = Category::Conflict;
                p.conflict_reason = Some(format!("Could not read the files to compare them: {e}"));
            }
        }
    }
    Ok(())
}

/// Byte-by-byte comparison in 1 MB blocks. Reports bytes read (per side) through `tick`.
pub fn files_identical(a: &Path, b: &Path, cancel: &CancelToken, tick: &mut dyn FnMut(u64)) -> std::io::Result<bool> {
    const BLOCK: usize = 1024 * 1024;
    let mut fa = File::open(a)?;
    let mut fb = File::open(b)?;
    if fa.metadata()?.len() != fb.metadata()?.len() {
        return Ok(false);
    }
    let mut ba = vec![0u8; BLOCK];
    let mut bb = vec![0u8; BLOCK];
    loop {
        if cancel.is_cancelled() {
            return Err(std::io::Error::new(ErrorKind::Interrupted, "cancelled"));
        }
        let na = read_full(&mut fa, &mut ba)?;
        let nb = read_full(&mut fb, &mut bb)?;
        if na != nb || ba[..na] != bb[..nb] {
            return Ok(false);
        }
        if na == 0 {
            return Ok(true);
        }
        tick(na as u64);
    }
}

/// Fills `buf` as far as possible (a single `read` may return less than asked).
fn read_full(f: &mut File, buf: &mut [u8]) -> std::io::Result<usize> {
    let mut filled = 0;
    while filled < buf.len() {
        match f.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => filled += n,
            Err(e) if e.kind() == ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        }
    }
    Ok(filled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::filter::FilterConfig;
    use std::fs;
    use std::time::{Duration, SystemTime};

    const BASE_TIME: u64 = 1_760_000_000; // a fixed date in 2025

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

    /// Writes a file and sets its modification time to BASE_TIME + `secs`.
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

    fn run(f: &Fixture, options: CompareOptions, filter: &Filter) -> FolderPairResult {
        let input = [FolderPairInput {
            left: f.left.display().to_string(),
            right: f.right.display().to_string(),
        }];
        compare(&input, &options, filter, &CancelToken::new(), &|_| {})
            .unwrap()
            .remove(0)
    }

    fn cat(r: &FolderPairResult, rel: &str) -> Category {
        r.pairs
            .iter()
            .find(|p| p.rel_path.eq_ignore_ascii_case(rel))
            .unwrap_or_else(|| {
                panic!(
                    "{rel} not found in {:?}",
                    r.pairs.iter().map(|p| &p.rel_path).collect::<Vec<_>>()
                )
            })
            .category
    }

    fn time_size() -> CompareOptions {
        CompareOptions::default()
    }

    #[test]
    fn identical_folders_are_equal() {
        let f = fixture();
        for base in [&f.left, &f.right] {
            write(base, "a.txt", b"hello", 0);
            write(base, r"sub\b.txt", b"world", 0);
        }
        let r = run(&f, time_size(), &Filter::none());
        assert!(r.pairs.iter().all(|p| p.category == Category::Equal), "{:?}", r.pairs);
        assert_eq!(r.stats.equal, 3); // a.txt, sub, sub\b.txt
        assert_eq!(r.stats.left_files, 2);
        assert_eq!(r.stats.left_folders, 1);
        assert_eq!(r.stats.left_bytes, 10);
    }

    #[test]
    fn one_sided_items() {
        let f = fixture();
        write(&f.left, r"only_left\deep\x.txt", b"1", 0);
        write(&f.right, "only_right.txt", b"2", 0);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(cat(&r, "only_left"), Category::LeftOnly);
        assert_eq!(cat(&r, r"only_left\deep"), Category::LeftOnly);
        assert_eq!(cat(&r, r"only_left\deep\x.txt"), Category::LeftOnly);
        assert_eq!(cat(&r, "only_right.txt"), Category::RightOnly);
    }

    #[test]
    fn newer_side_is_detected() {
        let f = fixture();
        write(&f.left, "l.txt", b"same", 100);
        write(&f.right, "l.txt", b"same", 0);
        write(&f.left, "r.txt", b"same", 0);
        write(&f.right, "r.txt", b"same!", 100);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(cat(&r, "l.txt"), Category::LeftNewer);
        assert_eq!(cat(&r, "r.txt"), Category::RightNewer);
    }

    #[test]
    fn two_second_tolerance_for_usb_drives() {
        let f = fixture();
        write(&f.left, "a.txt", b"x", 0);
        write(&f.right, "a.txt", b"x", 2);
        write(&f.left, "b.txt", b"x", 0);
        write(&f.right, "b.txt", b"x", 3);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(cat(&r, "a.txt"), Category::Equal);
        assert_eq!(cat(&r, "b.txt"), Category::RightNewer);
    }

    #[test]
    fn hour_shift_only_ignored_when_enabled() {
        let f = fixture();
        write(&f.left, "a.txt", b"x", 3600);
        write(&f.right, "a.txt", b"x", 0);
        assert_eq!(
            cat(&run(&f, time_size(), &Filter::none()), "a.txt"),
            Category::LeftNewer
        );
        let opts = CompareOptions {
            ignore_hour_shift: true,
            ..time_size()
        };
        assert_eq!(cat(&run(&f, opts, &Filter::none()), "a.txt"), Category::Equal);
    }

    #[test]
    fn same_time_different_size_is_a_conflict() {
        let f = fixture();
        write(&f.left, "a.txt", b"short", 0);
        write(&f.right, "a.txt", b"longer text", 0);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(cat(&r, "a.txt"), Category::Conflict);
        assert!(r.pairs[0].conflict_reason.is_some());
    }

    #[test]
    fn size_mode_ignores_time() {
        let f = fixture();
        write(&f.left, "a.txt", b"abc", 0);
        write(&f.right, "a.txt", b"xyz", 5000);
        write(&f.left, "b.txt", b"abc", 0);
        write(&f.right, "b.txt", b"abcd", 0);
        let opts = CompareOptions {
            variant: CompareVariant::Size,
            ..time_size()
        };
        let r = run(&f, opts, &Filter::none());
        assert_eq!(cat(&r, "a.txt"), Category::Equal);
        assert_eq!(cat(&r, "b.txt"), Category::Different);
    }

    #[test]
    fn content_mode_reads_bytes() {
        let f = fixture();
        // Same size, same time, different bytes: only content mode can see this.
        write(&f.left, "a.bin", b"AAAA", 0);
        write(&f.right, "a.bin", b"AAAB", 0);
        // Same bytes, different times: equal in content mode.
        write(&f.left, "b.bin", b"same", 0);
        write(&f.right, "b.bin", b"same", 9999);
        // Larger than one 1 MB block, differing only in the last byte.
        let mut big = vec![7u8; 3 * 1024 * 1024 + 5];
        write(&f.left, "big.bin", &big, 0);
        *big.last_mut().unwrap() = 8;
        write(&f.right, "big.bin", &big, 0);

        assert_eq!(cat(&run(&f, time_size(), &Filter::none()), "a.bin"), Category::Equal);
        let opts = CompareOptions {
            variant: CompareVariant::Content,
            ..time_size()
        };
        let r = run(&f, opts, &Filter::none());
        assert_eq!(cat(&r, "a.bin"), Category::Different);
        assert_eq!(cat(&r, "b.bin"), Category::Equal);
        assert_eq!(cat(&r, "big.bin"), Category::Different);
    }

    #[test]
    fn names_match_case_insensitively() {
        let f = fixture();
        write(&f.left, "ReadMe.TXT", b"x", 0);
        write(&f.right, "readme.txt", b"x", 0);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(r.pairs.len(), 1);
        assert_eq!(r.pairs[0].category, Category::Equal);
        assert_eq!(r.pairs[0].rel_path, "ReadMe.TXT");
    }

    #[test]
    fn file_versus_folder_is_a_conflict() {
        let f = fixture();
        write(&f.left, "thing", b"i am a file", 0);
        write(&f.right, r"thing\inside.txt", b"i am in a folder", 0);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(cat(&r, "thing"), Category::Conflict);
        assert_eq!(cat(&r, r"thing\inside.txt"), Category::RightOnly);
    }

    #[test]
    fn children_follow_their_parent_in_order() {
        let f = fixture();
        write(&f.left, r"a\z.txt", b"1", 0);
        write(&f.left, "a b.txt", b"1", 0);
        write(&f.left, "b.txt", b"1", 0);
        let r = run(&f, time_size(), &Filter::none());
        let order: Vec<&str> = r.pairs.iter().map(|p| p.rel_path.as_str()).collect();
        assert_eq!(order, vec!["a", r"a\z.txt", "a b.txt", "b.txt"]);
    }

    #[test]
    fn missing_target_folder_means_everything_is_new() {
        let f = fixture();
        write(&f.left, "a.txt", b"x", 0);
        let input = [FolderPairInput {
            left: f.left.display().to_string(),
            right: f.right.join("not-created-yet").display().to_string(),
        }];
        let r = compare(&input, &time_size(), &Filter::none(), &CancelToken::new(), &|_| {})
            .unwrap()
            .remove(0);
        assert!(r.left_exists);
        assert!(!r.right_exists);
        assert_eq!(cat(&r, "a.txt"), Category::LeftOnly);
    }

    #[test]
    fn both_folders_missing_is_an_error() {
        let f = fixture();
        let input = [FolderPairInput {
            left: f.left.join("nope1").display().to_string(),
            right: f.right.join("nope2").display().to_string(),
        }];
        let err = compare(&input, &time_size(), &Filter::none(), &CancelToken::new(), &|_| {}).unwrap_err();
        assert!(matches!(err, EngineError::BothFoldersMissing { .. }));
    }

    #[test]
    fn base_path_that_is_a_file_is_an_error() {
        let f = fixture();
        write(&f.left, "file.txt", b"x", 0);
        let input = [FolderPairInput {
            left: f.left.join("file.txt").display().to_string(),
            right: f.right.display().to_string(),
        }];
        let err = compare(&input, &time_size(), &Filter::none(), &CancelToken::new(), &|_| {}).unwrap_err();
        assert!(matches!(err, EngineError::NotAFolder { .. }));
    }

    #[test]
    fn filter_is_applied_while_scanning() {
        let f = fixture();
        for base in [&f.left, &f.right] {
            write(base, "keep.txt", b"x", 0);
        }
        write(&f.left, "junk.tmp", b"x", 0);
        write(&f.left, r"node_modules\pkg\index.js", b"x", 0);
        write(&f.left, ".gitignore", b"x", 0);
        let filter = Filter::new(&FilterConfig {
            include: vec!["*".into()],
            exclude: vec!["*.tmp".into(), "node_modules".into(), ".git".into()],
        });
        let r = run(&f, time_size(), &filter);
        let paths: Vec<&str> = r.pairs.iter().map(|p| p.rel_path.as_str()).collect();
        assert_eq!(paths, vec![".gitignore", "keep.txt"]);
    }

    #[test]
    fn unicode_and_long_paths() {
        let f = fixture();
        let marathi = "माझा डेटा सुरक्षित आहे.txt";
        write(&f.left, marathi, "नमस्कार".as_bytes(), 0);
        write(&f.right, marathi, "नमस्कार".as_bytes(), 0);
        // Build a path well over the old 260-character Windows limit.
        let long_rel: String = (0..12)
            .map(|i| format!("folder_with_a_long_name_{i:02}\\"))
            .collect::<String>()
            + "deep.txt";
        assert!(f.left.join(&long_rel).display().to_string().len() > 300);
        write(&f.left, &long_rel, b"deep", 0);
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(cat(&r, marathi), Category::Equal);
        assert_eq!(cat(&r, &long_rel), Category::LeftOnly);
        assert!(r.errors.is_empty(), "{:?}", r.errors);
    }

    #[test]
    fn cancel_stops_the_comparison() {
        let f = fixture();
        for i in 0..2000 {
            write(&f.left, &format!("f{i}.txt"), b"x", 0);
        }
        let cancel = CancelToken::new();
        cancel.cancel();
        let input = [FolderPairInput {
            left: f.left.display().to_string(),
            right: f.right.display().to_string(),
        }];
        let err = compare(&input, &time_size(), &Filter::none(), &cancel, &|_| {}).unwrap_err();
        assert_eq!(err, EngineError::Cancelled);
    }

    #[test]
    fn several_folder_pairs() {
        let f = fixture();
        let l2 = f.left.join("second");
        let r2 = f.right.join("second");
        write(&f.left, "one.txt", b"1", 0);
        write(&l2, "two.txt", b"2", 0);
        fs::create_dir_all(&r2).unwrap();
        let input = [
            FolderPairInput {
                left: f.left.join("one-only").display().to_string(),
                right: f.right.join("x").display().to_string(),
            },
            FolderPairInput {
                left: l2.display().to_string(),
                right: r2.display().to_string(),
            },
        ];
        // First pair: both missing -> whole compare fails, so test the second on its own too.
        assert!(compare(&input, &time_size(), &Filter::none(), &CancelToken::new(), &|_| {}).is_err());
        let r = compare(&input[1..], &time_size(), &Filter::none(), &CancelToken::new(), &|_| {}).unwrap();
        assert_eq!(cat(&r[0], "two.txt"), Category::LeftOnly);
    }

    #[cfg(windows)]
    #[test]
    fn junctions_are_listed_but_not_followed() {
        let f = fixture();
        write(&f.left, r"real\file.txt", b"x", 0);
        let link = f.left.join("loop");
        let status = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(&f.left)
            .output()
            .unwrap();
        assert!(status.status.success(), "{status:?}");
        let r = run(&f, time_size(), &Filter::none());
        assert_eq!(r.skipped_links, vec!["loop".to_string()]);
        assert!(r.pairs.iter().all(|p| !p.rel_path.starts_with("loop")));
    }
}
