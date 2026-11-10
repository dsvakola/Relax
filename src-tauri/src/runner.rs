//! Shared pieces used by both the window and unattended (scheduled) runs:
//! Relax's data folder, job files, sync logs, the Two-way database update, batch mode and
//! Windows Task Scheduler integration.

use crate::commands::JobConfig;
use crate::engine::compare::{compare, CompareOptions, CompareVariant, FolderPairInput};
use crate::engine::db::SyncDb;
use crate::engine::execute::{execute, DeletionPolicy, ErrorDecision, ExecOptions, ExecReport, PauseToken};
use crate::engine::filter::{Filter, FilterConfig};
use crate::engine::plan::{plan_pair, DirectionTable, PairPlan, SyncConfig, SyncVariant};
use crate::engine::{winfs, CancelToken};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Same folder Tauri uses for this app (`identifier` in tauri.conf.json).
pub fn data_dir() -> PathBuf {
    let base = std::env::var_os("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    base.join("in.edu.vsa.relax")
}

pub fn db_dir() -> PathBuf {
    data_dir().join("sync-db")
}

pub fn logs_dir() -> PathBuf {
    data_dir().join("logs")
}

const KEEP_LOGS: usize = 200;

// ---------------------------------------------------------------------------
// Job files (.relax), as written by the window (src/settings.ts)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JobFile {
    settings: JobSettings,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct JobSettings {
    pairs: Vec<FolderPairInput>,
    compare_variant: CompareVariant,
    time_tolerance: u32,
    ignore_hour_shift: bool,
    include: Vec<String>,
    exclude: Vec<String>,
    sync_variant: SyncVariant,
    custom: DirectionTable,
    allow_empty: bool,
    deletion: String,
    versions_folder: String,
    verify: bool,
}

impl Default for JobSettings {
    fn default() -> Self {
        let f = FilterConfig::default();
        Self {
            pairs: vec![],
            compare_variant: CompareVariant::TimeAndSize,
            time_tolerance: 2,
            ignore_hour_shift: false,
            include: f.include,
            exclude: f.exclude,
            sync_variant: SyncVariant::Mirror,
            custom: DirectionTable::mirror(),
            allow_empty: false,
            deletion: "recycleBin".into(),
            versions_folder: String::new(),
            verify: false,
        }
    }
}

/// Reads a `.relax` job file into the engine's configuration.
pub fn load_job(path: &Path) -> Result<JobConfig, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("Cannot read job file {}: {e}", path.display()))?;
    let file: JobFile =
        serde_json::from_str(&text).map_err(|e| format!("{} is not a valid Relax job: {e}", path.display()))?;
    let s = file.settings;
    let pairs: Vec<FolderPairInput> = s
        .pairs
        .into_iter()
        .filter(|p| !p.left.trim().is_empty() && !p.right.trim().is_empty())
        .collect();
    if pairs.is_empty() {
        return Err(format!("The job {} has no complete folder pair.", path.display()));
    }
    let deletion = match s.deletion.as_str() {
        "permanent" => DeletionPolicy::Permanent,
        "versioning" if !s.versions_folder.trim().is_empty() => DeletionPolicy::Versioning {
            folder: s.versions_folder.trim().to_string(),
        },
        _ => DeletionPolicy::RecycleBin,
    };
    Ok(JobConfig {
        pairs,
        compare: CompareOptions {
            variant: s.compare_variant,
            time_tolerance_secs: s.time_tolerance,
            ignore_hour_shift: s.ignore_hour_shift,
        },
        filter: FilterConfig {
            include: s.include,
            exclude: s.exclude,
        },
        sync: SyncConfig {
            variant: s.sync_variant,
            custom: s.custom,
            allow_sync_from_empty_folder: s.allow_empty,
            time_tolerance_secs: s.time_tolerance,
        },
        exec: ExecOptions {
            deletion,
            verify_copies: s.verify,
        },
    })
}

pub fn job_name(path: &Path) -> String {
    path.file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The filter of a job, plus its versions folder if that lies inside one of the synced folders.
pub fn make_filter(job: &JobConfig) -> Filter {
    let mut cfg = job.filter.clone();
    if let DeletionPolicy::Versioning { folder } = &job.exec.deletion {
        let f = folder.trim().trim_end_matches(['\\', '/']).to_lowercase();
        for fp in &job.pairs {
            for base in [&fp.left, &fp.right] {
                let b = base.trim().trim_end_matches(['\\', '/']).to_lowercase();
                if let Some(rel) = f.strip_prefix(&format!("{b}\\")) {
                    cfg.exclude.push(format!("\\{rel}\\"));
                }
            }
        }
    }
    Filter::new(&cfg)
}

/// Two-way: records the state right after a sync, for next time.
pub fn remember_two_way(job: &JobConfig, plan: &PairPlan) {
    if plan.variant != SyncVariant::TwoWay {
        return;
    }
    let input = [FolderPairInput {
        left: plan.left.clone(),
        right: plan.right.clone(),
    }];
    if let Ok(mut now) = compare(&input, &job.compare, &make_filter(job), &CancelToken::new(), &|_| {}) {
        let file = SyncDb::file_for_pair(&db_dir(), &plan.left, &plan.right);
        let old = SyncDb::load(&file);
        let _ = SyncDb::after_sync(old.as_ref(), &now.remove(0)).save(&file);
    }
}

// ---------------------------------------------------------------------------
// Sync logs
// ---------------------------------------------------------------------------

pub struct LogInput<'a> {
    pub job_name: &'a str,
    pub scheduled: bool,
    pub variant: SyncVariant,
    pub reports: &'a [ExecReport],
    pub skipped: &'a [String],
    pub cancelled: bool,
    pub failure: Option<&'a str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LogStatus {
    Ok,
    Problems,
    Cancelled,
    Failed,
}

impl LogStatus {
    fn label(self) -> &'static str {
        match self {
            LogStatus::Ok => "OK",
            LogStatus::Problems => "Problems",
            LogStatus::Cancelled => "Cancelled",
            LogStatus::Failed => "Failed",
        }
    }
}

pub fn log_status(input: &LogInput) -> LogStatus {
    if input.failure.is_some() {
        LogStatus::Failed
    } else if input.cancelled {
        LogStatus::Cancelled
    } else if !input.skipped.is_empty() || input.reports.iter().any(|r| !r.failures.is_empty()) {
        LogStatus::Problems
    } else {
        LogStatus::Ok
    }
}

pub fn format_bytes(b: u64) -> String {
    const U: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = b as f64;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{b} B")
    } else {
        format!("{v:.2} {}", U[i])
    }
}

fn variant_name(v: SyncVariant) -> &'static str {
    match v {
        SyncVariant::TwoWay => "Two-way",
        SyncVariant::Mirror => "Mirror",
        SyncVariant::Update => "Update (incremental)",
        SyncVariant::Custom => "Custom",
    }
}

/// Writes a readable log of one sync and returns its path. Keeps the newest 200 logs.
pub fn write_log(input: &LogInput) -> Option<PathBuf> {
    let status = log_status(input);
    let stamp = winfs::local_timestamp();
    let mut t = String::new();
    let _ = writeln!(t, "Relax - Your data is safe!  version {}", env!("CARGO_PKG_VERSION"));
    let _ = writeln!(t, "Vidyasagar Academy · https://vsa.edu.in/");
    let _ = writeln!(t, "==============================================================");
    let pretty = if stamp.len() == 17 {
        format!(
            "{} {}:{}:{}",
            &stamp[..10],
            &stamp[11..13],
            &stamp[13..15],
            &stamp[15..17]
        )
    } else {
        stamp.clone()
    };
    let _ = writeln!(t, "Date:     {pretty}");
    let _ = writeln!(
        t,
        "Job:      {}",
        if input.job_name.is_empty() {
            "(not saved)"
        } else {
            input.job_name
        }
    );
    let _ = writeln!(
        t,
        "Started:  {}",
        if input.scheduled {
            "automatically (scheduled)"
        } else {
            "by you"
        }
    );
    let _ = writeln!(t, "Mode:     {}", variant_name(input.variant));
    let _ = writeln!(t);
    for (i, r) in input.reports.iter().enumerate() {
        let d = &r.done;
        let _ = writeln!(t, "Folder pair {}:  {}  ->  {}", i + 1, r.left, r.right);
        let _ = writeln!(
            t,
            "  New items copied: {}   Overwritten: {}   Deleted: {}   Data copied: {}   Time: {:.1} s",
            d.create_left + d.create_right,
            d.update_left + d.update_right,
            d.delete_left + d.delete_right,
            format_bytes(r.bytes_copied),
            r.duration_ms as f64 / 1000.0
        );
        for n in &r.notes {
            let _ = writeln!(t, "  Note: {n}");
        }
        if !r.failures.is_empty() {
            let _ = writeln!(t, "  Problems ({}):", r.failures.len());
            for f in &r.failures {
                let _ = writeln!(t, "    - {}: {}", f.rel_path, f.message);
            }
        }
        if r.cancelled {
            let _ = writeln!(t, "  Cancelled before finishing.");
        }
        let _ = writeln!(t);
    }
    for s in input.skipped {
        let _ = writeln!(t, "Skipped (not synchronized): {s}");
    }
    if let Some(f) = input.failure {
        let _ = writeln!(t, "Error: {f}");
    }
    let _ = writeln!(t);
    let _ = writeln!(
        t,
        "Result: {}",
        match status {
            LogStatus::Ok => "Completed successfully",
            LogStatus::Problems => "Completed with problems (see above)",
            LogStatus::Cancelled => "Cancelled",
            LogStatus::Failed => "Failed",
        }
    );

    let dir = logs_dir();
    fs::create_dir_all(&dir).ok()?;
    let job = if input.job_name.is_empty() {
        String::new()
    } else {
        format!(" {}", sanitize_file_name(input.job_name))
    };
    let mut path = dir.join(format!("{stamp}{job} - {}.txt", status.label()));
    let mut n = 2;
    while path.exists() {
        path = dir.join(format!("{stamp}{job} ({n}) - {}.txt", status.label()));
        n += 1;
    }
    // Windows Notepad and others read UTF-8 with a BOM reliably.
    let mut bytes = vec![0xEF, 0xBB, 0xBF];
    bytes.extend_from_slice(t.replace('\n', "\r\n").as_bytes());
    fs::write(&path, bytes).ok()?;
    prune_logs(&dir);
    Some(path)
}

fn sanitize_file_name(s: &str) -> String {
    s.chars()
        .map(|c| {
            if r#"<>:"/\|?*"#.contains(c) || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect()
}

fn prune_logs(dir: &Path) {
    let mut names = list_log_names(dir);
    if names.len() <= KEEP_LOGS {
        return;
    }
    names.sort();
    for old in &names[..names.len() - KEEP_LOGS] {
        let _ = fs::remove_file(dir.join(old));
    }
}

fn list_log_names(dir: &Path) -> Vec<String> {
    fs::read_dir(dir)
        .map(|it| {
            it.filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| n.ends_with(".txt"))
                .collect()
        })
        .unwrap_or_default()
}

/// Log file names, newest first.
pub fn list_logs() -> Vec<String> {
    let mut names = list_log_names(&logs_dir());
    names.sort();
    names.reverse();
    names
}

pub fn read_log(name: &str) -> Result<String, String> {
    if name.contains(['\\', '/']) || name.contains("..") || !name.ends_with(".txt") {
        return Err("Invalid log name.".into());
    }
    let bytes = fs::read(logs_dir().join(name)).map_err(|e| e.to_string())?;
    let text = String::from_utf8_lossy(&bytes);
    Ok(text.trim_start_matches('\u{feff}').replace("\r\n", "\n"))
}

// ---------------------------------------------------------------------------
// Batch mode: Relax.exe --run "job.relax"
// ---------------------------------------------------------------------------

/// Runs a job without a window (for Windows Task Scheduler). Errors are skipped and logged.
/// Exit code: 0 = success, 1 = finished with problems, 2 = failed.
pub fn run_batch(job_path: &Path) -> i32 {
    let name = job_name(job_path);
    let job = match load_job(job_path) {
        Ok(j) => j,
        Err(e) => {
            write_log(&LogInput {
                job_name: &name,
                scheduled: true,
                variant: SyncVariant::Mirror,
                reports: &[],
                skipped: &[],
                cancelled: false,
                failure: Some(&e),
            });
            return 2;
        }
    };
    let (reports, skipped, failure) = run_job(&job);
    let input = LogInput {
        job_name: &name,
        scheduled: true,
        variant: job.sync.variant,
        reports: &reports,
        skipped: &skipped,
        cancelled: false,
        failure: failure.as_deref(),
    };
    write_log(&input);
    match log_status(&input) {
        LogStatus::Ok => 0,
        LogStatus::Problems => 1,
        LogStatus::Cancelled | LogStatus::Failed => 2,
    }
}

/// Compare, plan and synchronize every folder pair of a job, without asking anything.
pub fn run_job(job: &JobConfig) -> (Vec<ExecReport>, Vec<String>, Option<String>) {
    let cancel = CancelToken::new();
    let filter = make_filter(job);
    let results = match compare(&job.pairs, &job.compare, &filter, &cancel, &|_| {}) {
        Ok(r) => r,
        Err(e) => return (vec![], vec![], Some(e.to_string())),
    };
    let mut reports = Vec::new();
    let mut skipped = Vec::new();
    for r in &results {
        let db = (job.sync.variant == SyncVariant::TwoWay)
            .then(|| SyncDb::load(&SyncDb::file_for_pair(&db_dir(), &r.left, &r.right)))
            .flatten();
        let plan = plan_pair(r, &job.sync, db.as_ref());
        if let Some(reason) = &plan.blocked {
            skipped.push(format!("{} -> {}: {reason}", plan.left, plan.right));
            continue;
        }
        if !plan.has_changes() {
            reports.push(ExecReport {
                left: plan.left.clone(),
                right: plan.right.clone(),
                notes: vec!["Already up to date.".into()],
                ..Default::default()
            });
            continue;
        }
        match execute(
            &plan,
            &job.exec,
            &cancel,
            &PauseToken::new(),
            &winfs::free_space,
            &|_| {},
            &|_| ErrorDecision::Ignore,
        ) {
            Ok(rep) => {
                remember_two_way(job, &plan);
                reports.push(rep);
            }
            Err(e) => skipped.push(format!("{} -> {}: {e}", plan.left, plan.right)),
        }
    }
    (reports, skipped, None)
}

// ---------------------------------------------------------------------------
// Windows Task Scheduler
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Schedule {
    /// "daily", "weekly" or "logon".
    pub kind: String,
    /// "HH:MM" (daily / weekly).
    pub time: String,
    /// "MON" ... "SUN" (weekly).
    pub day: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleInfo {
    pub exists: bool,
    pub next_run: String,
    pub task_name: String,
}

fn task_name(job_path: &Path) -> String {
    format!("Relax\\{}", sanitize_file_name(&job_name(job_path)))
}

fn schtasks(args: &[&str]) -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let out = Command::new("schtasks.exe")
        .args(args)
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Cannot start Windows Task Scheduler: {e}"))?;
    let text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.success() {
        Ok(text)
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { text.trim().to_string() } else { err })
    }
}

/// Name of the command-line alias declared in the Microsoft Store (MSIX) package manifest.
pub const STORE_ALIAS: &str = "RelaxBackup.exe";

/// The program Task Scheduler should start. A Microsoft Store (MSIX) install lives in the protected
/// WindowsApps folder, which Task Scheduler cannot start directly, so the package's alias is used instead.
fn scheduler_exe() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    if exe.display().to_string().to_lowercase().contains(r"\windowsapps\") {
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local)
                .join("Microsoft")
                .join("WindowsApps")
                .join(STORE_ALIAS));
        }
    }
    Ok(exe)
}

pub fn create_schedule(job_path: &Path, s: &Schedule) -> Result<ScheduleInfo, String> {
    if !job_path.is_file() {
        return Err("Please save the job first.".into());
    }
    let exe = scheduler_exe()?;
    let action = format!("\"{}\" --run \"{}\"", exe.display(), job_path.display());
    if action.len() > 261 {
        return Err(
            "The program and job paths are too long for Windows Task Scheduler. Save the job in a shorter folder."
                .into(),
        );
    }
    let valid_time = s.time.len() == 5
        && s.time.as_bytes()[2] == b':'
        && s.time[..2].parse::<u8>().is_ok_and(|h| h < 24)
        && s.time[3..].parse::<u8>().is_ok_and(|m| m < 60);
    let name = task_name(job_path);
    let mut args: Vec<&str> = vec!["/Create", "/F", "/TN", &name, "/TR", &action];
    match s.kind.as_str() {
        "daily" if valid_time => args.extend(["/SC", "DAILY", "/ST", &s.time]),
        "weekly" if valid_time && ["MON", "TUE", "WED", "THU", "FRI", "SAT", "SUN"].contains(&s.day.as_str()) => {
            args.extend(["/SC", "WEEKLY", "/D", &s.day, "/ST", &s.time])
        }
        "logon" => args.extend(["/SC", "ONLOGON"]),
        _ => return Err("Please choose a valid schedule.".into()),
    }
    schtasks(&args)?;
    get_schedule(job_path)
}

pub fn delete_schedule(job_path: &Path) -> Result<(), String> {
    schtasks(&["/Delete", "/F", "/TN", &task_name(job_path)]).map(|_| ())
}

pub fn get_schedule(job_path: &Path) -> Result<ScheduleInfo, String> {
    let name = task_name(job_path);
    match schtasks(&["/Query", "/TN", &name, "/FO", "LIST"]) {
        Ok(text) => {
            let next_run = text
                .lines()
                .find(|l| l.trim_start().starts_with("Next Run Time"))
                .and_then(|l| l.split_once(':'))
                .map(|(_, v)| v.trim().to_string())
                .unwrap_or_default();
            Ok(ScheduleInfo {
                exists: true,
                next_run,
                task_name: name,
            })
        }
        Err(_) => Ok(ScheduleInfo {
            exists: false,
            next_run: String::new(),
            task_name: name,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_file_from_window_is_understood() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("Daily.relax");
        fs::write(
            &p,
            r#"{"app":"Relax - Your data is safe!","format":1,"settings":{
              "pairs":[{"left":"D:\\Data","right":"F:\\Backup"},{"left":"","right":""}],
              "compareVariant":"content","timeTolerance":3,"ignoreHourShift":true,
              "include":["*"],"exclude":["*.tmp"],"syncVariant":"update",
              "custom":{"leftOnly":"toRight","rightOnly":"none","leftNewer":"toRight","rightNewer":"none","different":"toRight","conflict":"none"},
              "allowEmpty":false,"deletion":"versioning","versionsFolder":"F:\\Versions","verify":true,"unknownField":1}}"#,
        )
        .unwrap();
        let job = load_job(&p).unwrap();
        assert_eq!(job.pairs.len(), 1, "incomplete pairs are dropped");
        assert_eq!(job.compare.variant, CompareVariant::Content);
        assert_eq!(job.compare.time_tolerance_secs, 3);
        assert_eq!(job.sync.variant, SyncVariant::Update);
        assert_eq!(job.filter.exclude, vec!["*.tmp".to_string()]);
        assert_eq!(
            job.exec.deletion,
            DeletionPolicy::Versioning {
                folder: r"F:\Versions".into()
            }
        );
        assert!(job.exec.verify_copies);
        assert_eq!(job_name(&p), "Daily");
    }

    #[test]
    fn bad_job_files_are_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let p = tmp.path().join("x.relax");
        fs::write(&p, "not json").unwrap();
        assert!(load_job(&p).is_err());
        fs::write(&p, r#"{"settings":{"pairs":[]}}"#).unwrap();
        assert!(load_job(&p).unwrap_err().contains("no complete folder pair"));
    }

    #[test]
    fn run_job_backs_up_incrementally() {
        let tmp = tempfile::tempdir().unwrap();
        let (src, dst) = (tmp.path().join("src"), tmp.path().join("dst"));
        fs::create_dir_all(src.join("sub")).unwrap();
        fs::write(src.join("a.txt"), b"a").unwrap();
        fs::write(src.join("sub").join("b.txt"), b"b").unwrap();
        let job = JobConfig {
            pairs: vec![FolderPairInput {
                left: src.display().to_string(),
                right: dst.display().to_string(),
            }],
            sync: SyncConfig {
                variant: SyncVariant::Update,
                ..Default::default()
            },
            exec: ExecOptions {
                deletion: DeletionPolicy::Permanent,
                verify_copies: false,
            },
            ..Default::default()
        };
        let (reports, skipped, failure) = run_job(&job);
        assert!(failure.is_none() && skipped.is_empty());
        assert_eq!(reports[0].done.create_right, 3);
        assert_eq!(fs::read(dst.join("sub").join("b.txt")).unwrap(), b"b");
        // Second run: nothing to do.
        let (reports, _, _) = run_job(&job);
        assert_eq!(reports[0].done.create_right + reports[0].done.update_right, 0);
        assert!(reports[0].notes.iter().any(|n| n.contains("up to date")));
    }

    #[test]
    fn log_status_and_sizes() {
        let ok = LogInput {
            job_name: "",
            scheduled: false,
            variant: SyncVariant::Mirror,
            reports: &[],
            skipped: &[],
            cancelled: false,
            failure: None,
        };
        assert_eq!(log_status(&ok), LogStatus::Ok);
        let skipped = vec!["x".to_string()];
        assert_eq!(
            log_status(&LogInput {
                skipped: &skipped,
                ..ok
            }),
            LogStatus::Problems
        );
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1536), "1.50 KB");
        assert_eq!(sanitize_file_name("a/b:c"), "a_b_c");
    }

    #[test]
    fn read_log_rejects_paths() {
        assert!(read_log(r"..\secret.txt").is_err());
        assert!(read_log("x.exe").is_err());
    }
}
