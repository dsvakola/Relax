//! Tauri commands: the bridge between the screens (TypeScript) and the sync engine (Rust).
//! Long operations run on a background thread and report progress through a `Channel`.

use crate::engine::compare::{
    compare, Category, CompareOptions, CompareProgress, CompareStats, FolderPairInput, FolderPairResult,
};
use crate::engine::db::SyncDb;
use crate::engine::execute::{execute, ErrorDecision, ExecOptions, ExecProgress, ExecReport, ItemFailure, PauseToken};
use crate::engine::filter::FilterConfig;
use crate::engine::plan::{
    check_folder_pairs, finalize, plan_pair, table_action, Action, Direction, PairPlan, PlanStats, PlannedItem,
    SyncConfig, SyncVariant,
};
use crate::engine::scan::{FsItem, ItemKind};
use crate::engine::{winfs, CancelToken, EngineError, ItemError};
use crate::runner::{self, make_filter};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::Channel;
use tauri::{AppHandle, Manager, State};

/// Everything the user configured for one sync job.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct JobConfig {
    pub pairs: Vec<FolderPairInput>,
    pub compare: CompareOptions,
    pub filter: FilterConfig,
    pub sync: SyncConfig,
    pub exec: ExecOptions,
}

/// Which rows the preview grid shows.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ViewFilter {
    pub show_equal: bool,
    pub show_skipped: bool,
    pub show_create: bool,
    pub show_update: bool,
    pub show_delete: bool,
    pub show_conflict: bool,
}

impl Default for ViewFilter {
    fn default() -> Self {
        Self {
            show_equal: false,
            show_skipped: true,
            show_create: true,
            show_update: true,
            show_delete: true,
            show_conflict: true,
        }
    }
}

impl ViewFilter {
    fn shows(&self, item: &PlannedItem) -> bool {
        match item.action {
            Action::None if item.pair.category == Category::Equal => self.show_equal,
            Action::None => self.show_skipped,
            Action::CreateLeft | Action::CreateRight => self.show_create,
            Action::UpdateLeft | Action::UpdateRight => self.show_update,
            Action::DeleteLeft | Action::DeleteRight => self.show_delete,
            Action::Conflict => self.show_conflict,
        }
    }
}

#[derive(Clone, Copy)]
enum RowRef {
    Header(usize),
    Item(usize, usize),
}

#[derive(Default)]
struct Session {
    job: JobConfig,
    results: Vec<FolderPairResult>,
    plans: Vec<PairPlan>,
    view_filter: ViewFilter,
    view: Vec<RowRef>,
}

#[derive(Default)]
struct OpControl {
    cancel: Option<CancelToken>,
    pause: Option<PauseToken>,
    error_tx: Option<mpsc::Sender<ErrorDecision>>,
}

#[derive(Default)]
pub struct AppState {
    session: Mutex<Session>,
    op: Mutex<OpControl>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairSummary {
    pub left: String,
    pub right: String,
    pub left_exists: bool,
    pub right_exists: bool,
    pub variant: SyncVariant,
    pub stats: PlanStats,
    pub compare_stats: CompareStats,
    pub warnings: Vec<String>,
    pub blocked: Option<String>,
    /// First 200 read errors.
    pub errors: Vec<ItemError>,
    pub error_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub pairs: Vec<PairSummary>,
    pub warnings: Vec<String>,
    pub row_count: usize,
    pub can_sync: bool,
    pub elapsed_ms: u64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Cell {
    pub name: String,
    pub size: u64,
    pub modified_ms: i64,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GridRow {
    /// `pair_index * 2^32 + item_index`; header rows use item index 2^32 - 1.
    pub id: u64,
    pub header: bool,
    pub pair: usize,
    pub path: String,
    pub depth: usize,
    pub kind: Option<ItemKind>,
    pub left: Option<Cell>,
    pub right: Option<Cell>,
    pub category: Option<Category>,
    pub action: Option<Action>,
    pub note: Option<String>,
}

const HEADER_INDEX: u64 = u32::MAX as u64;

fn cell(item: &Option<FsItem>) -> Option<Cell> {
    item.as_ref().map(|i| Cell {
        name: i.rel_path.rsplit('\\').next().unwrap_or(&i.rel_path).to_string(),
        size: i.size,
        modified_ms: i.modified.div_euclid(10_000),
    })
}

fn rebuild_view(s: &mut Session) {
    let multi = s.plans.len() > 1;
    let mut view = Vec::new();
    for (pi, plan) in s.plans.iter().enumerate() {
        if multi {
            view.push(RowRef::Header(pi));
        }
        // Folders hidden by the filter are still shown when something inside them is visible,
        // so every row keeps its place in the tree. `open` holds (depth, index, already shown).
        let mut open: Vec<(usize, usize, bool)> = Vec::new();
        for (ii, item) in plan.items.iter().enumerate() {
            let depth = item.pair.rel_path.matches('\\').count();
            while open.last().is_some_and(|(d, _, _)| *d >= depth) {
                open.pop();
            }
            let shown = s.view_filter.shows(item);
            if shown {
                for (_, fi, done) in open.iter_mut().filter(|(_, _, done)| !*done) {
                    view.push(RowRef::Item(pi, *fi));
                    *done = true;
                }
                view.push(RowRef::Item(pi, ii));
            }
            let is_folder = item
                .pair
                .left
                .as_ref()
                .or(item.pair.right.as_ref())
                .is_some_and(|i| i.kind == ItemKind::Folder);
            if is_folder {
                open.push((depth, ii, shown));
            }
        }
    }
    s.view = view;
}

fn summary(s: &Session, elapsed_ms: u64) -> Summary {
    let pairs: Vec<PairSummary> = s
        .plans
        .iter()
        .zip(&s.results)
        .map(|(p, r)| PairSummary {
            left: p.left.clone(),
            right: p.right.clone(),
            left_exists: r.left_exists,
            right_exists: r.right_exists,
            variant: p.variant,
            stats: p.stats.clone(),
            compare_stats: r.stats.clone(),
            warnings: p.warnings.clone(),
            blocked: p.blocked.clone(),
            errors: r.errors.iter().take(200).cloned().collect(),
            error_count: r.errors.len(),
        })
        .collect();
    let can_sync = s.plans.iter().any(|p| p.blocked.is_none() && p.has_changes());
    Summary {
        pairs,
        warnings: check_folder_pairs(&s.job.pairs),
        row_count: s.view.len(),
        can_sync,
        elapsed_ms,
    }
}

/// Throttles progress messages to the screen (at most ~10 per second).
struct Throttle(Mutex<Instant>);

impl Throttle {
    fn new() -> Self {
        Self(Mutex::new(Instant::now() - Duration::from_secs(1)))
    }

    fn ready(&self) -> bool {
        let mut last = self.0.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() >= Duration::from_millis(100) {
            *last = Instant::now();
            true
        } else {
            false
        }
    }
}

fn begin_operation(state: &AppState) -> (CancelToken, PauseToken) {
    let mut op = state.op.lock().unwrap_or_else(|e| e.into_inner());
    let cancel = CancelToken::new();
    let pause = PauseToken::new();
    op.cancel = Some(cancel.clone());
    op.pause = Some(pause.clone());
    op.error_tx = None;
    (cancel, pause)
}

#[tauri::command]
pub async fn compare_folders(
    app: AppHandle,
    job: JobConfig,
    on_progress: Channel<CompareProgress>,
) -> Result<Summary, String> {
    let state = app.state::<AppState>();
    let (cancel, _) = begin_operation(&state);
    let started = Instant::now();
    let db_dir = runner::db_dir();
    let job_for_thread = job.clone();

    let (results, plans) = tauri::async_runtime::spawn_blocking(move || {
        let job = job_for_thread;
        let filter = make_filter(&job);
        let throttle = Throttle::new();
        let progress = |p: CompareProgress| {
            if throttle.ready() {
                let _ = on_progress.send(p);
            }
        };
        let results = compare(&job.pairs, &job.compare, &filter, &cancel, &progress)?;
        let plans: Vec<PairPlan> = results
            .iter()
            .map(|r| {
                let db = (job.sync.variant == SyncVariant::TwoWay)
                    .then(|| SyncDb::load(&SyncDb::file_for_pair(&db_dir, &r.left, &r.right)))
                    .flatten();
                plan_pair(r, &job.sync, db.as_ref())
            })
            .collect();
        Ok::<_, EngineError>((results, plans))
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;

    let mut s = state.session.lock().unwrap_or_else(|e| e.into_inner());
    s.job = job;
    s.results = results;
    s.plans = plans;
    rebuild_view(&mut s);
    Ok(summary(&s, started.elapsed().as_millis() as u64))
}

#[tauri::command]
pub fn set_view(state: State<'_, AppState>, view: ViewFilter) -> usize {
    let mut s = state.session.lock().unwrap_or_else(|e| e.into_inner());
    s.view_filter = view;
    rebuild_view(&mut s);
    s.view.len()
}

#[tauri::command]
pub fn get_rows(state: State<'_, AppState>, offset: usize, limit: usize) -> Vec<GridRow> {
    let s = state.session.lock().unwrap_or_else(|e| e.into_inner());
    s.view
        .iter()
        .skip(offset)
        .take(limit.min(2000))
        .map(|r| match *r {
            RowRef::Header(pi) => {
                let p = &s.plans[pi];
                GridRow {
                    id: ((pi as u64) << 32) | HEADER_INDEX,
                    header: true,
                    pair: pi,
                    path: format!("{}  ⟶  {}", p.left, p.right),
                    depth: 0,
                    kind: None,
                    left: None,
                    right: None,
                    category: None,
                    action: None,
                    note: p.blocked.clone(),
                }
            }
            RowRef::Item(pi, ii) => {
                let it = &s.plans[pi].items[ii];
                GridRow {
                    id: ((pi as u64) << 32) | ii as u64,
                    header: false,
                    pair: pi,
                    path: it.pair.rel_path.clone(),
                    depth: it.pair.rel_path.matches('\\').count(),
                    kind: it.pair.left.as_ref().or(it.pair.right.as_ref()).map(|i| i.kind),
                    left: cell(&it.pair.left),
                    right: cell(&it.pair.right),
                    category: Some(it.pair.category),
                    action: Some(it.action),
                    note: it.note.clone().or_else(|| it.pair.conflict_reason.clone()),
                }
            }
        })
        .collect()
}

/// Changes the action of the selected rows (right-click menu). Safety rules are applied again.
#[tauri::command]
pub fn set_direction(
    state: State<'_, AppState>,
    ids: Vec<u64>,
    all_visible: Option<bool>,
    direction: Direction,
) -> Summary {
    let mut s = state.session.lock().unwrap_or_else(|e| e.into_inner());
    let mut touched: Vec<usize> = Vec::new();
    // "Select all" applies to every row currently shown, without sending all ids from the screen.
    let ids: Vec<u64> = if all_visible == Some(true) {
        s.view
            .iter()
            .filter_map(|r| match *r {
                RowRef::Item(pi, ii) => Some(((pi as u64) << 32) | ii as u64),
                RowRef::Header(_) => None,
            })
            .collect()
    } else {
        ids
    };
    for id in ids {
        let (pi, ii) = ((id >> 32) as usize, (id & HEADER_INDEX) as usize);
        let Some(plan) = s.plans.get_mut(pi) else { continue };
        let Some(item) = plan.items.get_mut(ii) else { continue };
        item.action = table_action(&item.pair, direction);
        item.note = Some("Changed by you.".into());
        if !touched.contains(&pi) {
            touched.push(pi);
        }
    }
    for pi in touched {
        let items = std::mem::take(&mut s.plans[pi].items);
        let refreshed = finalize(&s.results[pi], &s.job.sync, items);
        s.plans[pi] = refreshed;
    }
    rebuild_view(&mut s);
    summary(&s, 0)
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase", rename_all_fields = "camelCase", tag = "type")]
pub enum SyncEvent {
    Progress {
        pair_index: usize,
        pair_count: usize,
        progress: ExecProgress,
    },
    Error {
        pair_index: usize,
        failure: ItemFailure,
    },
    PairDone {
        pair_index: usize,
        report: ExecReport,
    },
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SyncOutcome {
    pub reports: Vec<ExecReport>,
    /// Folder pairs that were not synced, with the reason.
    pub skipped: Vec<String>,
    pub cancelled: bool,
    /// Error that stopped the sync, if any.
    pub failure: Option<String>,
    /// Where the log of this sync was saved.
    pub log_file: Option<String>,
}

#[tauri::command]
pub async fn start_sync(
    app: AppHandle,
    exec: ExecOptions,
    job_name: Option<String>,
    on_event: Channel<SyncEvent>,
) -> Result<SyncOutcome, String> {
    let state = app.state::<AppState>();
    let (cancel, pause) = begin_operation(&state);
    let (plans, job) = {
        let s = state.session.lock().unwrap_or_else(|e| e.into_inner());
        (s.plans.clone(), s.job.clone())
    };
    if plans.is_empty() {
        return Err("Please click Compare first.".into());
    }
    let app2 = app.clone();

    tauri::async_runtime::spawn_blocking(move || {
        let state = app2.state::<AppState>();
        let mut outcome = SyncOutcome {
            reports: Vec::new(),
            skipped: Vec::new(),
            cancelled: false,
            failure: None,
            log_file: None,
        };
        let pair_count = plans.len();
        for (pi, plan) in plans.iter().enumerate() {
            if cancel.is_cancelled() {
                outcome.cancelled = true;
                break;
            }
            if let Some(reason) = &plan.blocked {
                outcome
                    .skipped
                    .push(format!("{} ⟶ {}: {reason}", plan.left, plan.right));
                continue;
            }
            if !plan.has_changes() {
                continue;
            }
            let throttle = Throttle::new();
            let on_progress = |p: &ExecProgress| {
                if throttle.ready() || p.items_done == p.items_total {
                    let _ = on_event.send(SyncEvent::Progress {
                        pair_index: pi,
                        pair_count,
                        progress: p.clone(),
                    });
                }
            };
            let on_error = |f: &ItemFailure| {
                let (tx, rx) = mpsc::channel();
                state.op.lock().unwrap_or_else(|e| e.into_inner()).error_tx = Some(tx);
                let _ = on_event.send(SyncEvent::Error {
                    pair_index: pi,
                    failure: f.clone(),
                });
                loop {
                    match rx.recv_timeout(Duration::from_millis(200)) {
                        Ok(d) => return d,
                        Err(mpsc::RecvTimeoutError::Timeout) if !cancel.is_cancelled() => continue,
                        Err(_) => return ErrorDecision::Cancel,
                    }
                }
            };
            let report = match execute(
                plan,
                &exec,
                &cancel,
                &pause,
                &winfs::free_space,
                &on_progress,
                &on_error,
            ) {
                Ok(r) => r,
                Err(e) => {
                    outcome.failure = Some(format!("{} ⟶ {}: {e}", plan.left, plan.right));
                    break;
                }
            };
            runner::remember_two_way(&job, plan);

            let cancelled = report.cancelled;
            let _ = on_event.send(SyncEvent::PairDone {
                pair_index: pi,
                report: report.clone(),
            });
            outcome.reports.push(report);
            if cancelled {
                outcome.cancelled = true;
                break;
            }
        }
        outcome.log_file = runner::write_log(&runner::LogInput {
            job_name: job_name.as_deref().unwrap_or(""),
            scheduled: false,
            variant: job.sync.variant,
            reports: &outcome.reports,
            skipped: &outcome.skipped,
            cancelled: outcome.cancelled,
            failure: outcome.failure.as_deref(),
        })
        .map(|p| p.display().to_string());
        Ok::<_, String>(outcome)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn answer_error(state: State<'_, AppState>, decision: ErrorDecision) {
    if let Some(tx) = state.op.lock().unwrap_or_else(|e| e.into_inner()).error_tx.take() {
        let _ = tx.send(decision);
    }
}

#[tauri::command]
pub fn set_paused(state: State<'_, AppState>, paused: bool) {
    if let Some(p) = &state.op.lock().unwrap_or_else(|e| e.into_inner()).pause {
        p.set_paused(paused);
    }
}

#[tauri::command]
pub fn cancel_operation(state: State<'_, AppState>) {
    let op = state.op.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(c) = &op.cancel {
        c.cancel();
    }
    if let Some(p) = &op.pause {
        p.set_paused(false);
    }
}

/// Opens a folder (or the folder containing a file) in Windows Explorer.
#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let p = PathBuf::from(&path);
    if p.is_dir() {
        app.opener().open_path(path, None::<&str>).map_err(|e| e.to_string())
    } else {
        app.opener().reveal_item_in_dir(p).map_err(|e| e.to_string())
    }
}

/// Job files are small JSON documents with the `.relax` extension.
fn check_job_path(path: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(path);
    let ok = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("relax"));
    if ok {
        Ok(p)
    } else {
        Err("Relax job files must end with .relax".into())
    }
}

#[tauri::command]
pub fn load_job_file(path: String) -> Result<String, String> {
    let p = check_job_path(&path)?;
    let meta = std::fs::metadata(&p).map_err(|e| format!("Cannot open {path}: {e}"))?;
    if meta.len() > 5 * 1024 * 1024 {
        return Err(format!("{path} is too large to be a Relax job file."));
    }
    std::fs::read_to_string(&p).map_err(|e| format!("Cannot read {path}: {e}"))
}

#[tauri::command]
pub fn save_job_file(path: String, content: String) -> Result<(), String> {
    let p = check_job_path(&path)?;
    let tmp = p.with_extension("relax.tmp");
    std::fs::write(&tmp, content.as_bytes())
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("Cannot save {path}: {e}")
        })
}

#[tauri::command]
pub fn list_logs() -> Vec<String> {
    runner::list_logs()
}

#[tauri::command]
pub fn read_log(name: String) -> Result<String, String> {
    runner::read_log(&name)
}

#[tauri::command]
pub fn open_logs_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = runner::logs_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    app.opener()
        .open_path(dir.display().to_string(), None::<&str>)
        .map_err(|e| e.to_string())
}

#[tauri::command]
pub fn get_schedule(job_path: String) -> Result<runner::ScheduleInfo, String> {
    runner::get_schedule(&check_job_path(&job_path)?)
}

#[tauri::command]
pub fn create_schedule(job_path: String, schedule: runner::Schedule) -> Result<runner::ScheduleInfo, String> {
    runner::create_schedule(&check_job_path(&job_path)?, &schedule)
}

#[tauri::command]
pub fn delete_schedule(job_path: String) -> Result<(), String> {
    runner::delete_schedule(&check_job_path(&job_path)?)
}

/// A `.relax` file given on the command line (e.g. by double-clicking it in Explorer).
#[tauri::command]
pub fn startup_job() -> Option<String> {
    std::env::args()
        .skip(1)
        .find(|a| a.to_lowercase().ends_with(".relax") && std::path::Path::new(a).is_file())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobEntry {
    pub path: String,
    pub name: String,
    pub modified_ms: i64,
}

/// Where new jobs are saved by default: Documents\Relax Jobs.
#[tauri::command]
pub fn default_jobs_dir(app: AppHandle) -> Result<String, String> {
    let dir = app.path().document_dir().map_err(|e| e.to_string())?.join("Relax Jobs");
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    Ok(dir.display().to_string())
}

/// Saved jobs: every .relax file in the default jobs folder plus the given known paths that still exist.
#[tauri::command]
pub fn list_jobs(app: AppHandle, known: Vec<String>) -> Vec<JobEntry> {
    let mut paths: Vec<PathBuf> = known.into_iter().map(PathBuf::from).collect();
    if let Ok(dir) = app.path().document_dir().map(|d| d.join("Relax Jobs")) {
        if let Ok(rd) = std::fs::read_dir(dir) {
            paths.extend(rd.filter_map(|e| e.ok()).map(|e| e.path()));
        }
    }
    let mut seen = std::collections::HashSet::new();
    let mut out: Vec<JobEntry> = paths
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("relax")) && p.is_file())
        .filter(|p| seen.insert(p.display().to_string().to_lowercase()))
        .map(|p| JobEntry {
            name: runner::job_name(&p),
            modified_ms: std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .map(crate::engine::scan::system_time_to_ticks)
                .map(|t| t.div_euclid(10_000))
                .unwrap_or(0),
            path: p.display().to_string(),
        })
        .collect();
    out.sort_by_key(|j| j.name.to_lowercase());
    out
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RenameResult {
    pub path: String,
    /// The job had a schedule; it was removed because it pointed to the old file name.
    pub schedule_removed: bool,
}

#[tauri::command]
pub fn rename_job_file(path: String, new_name: String) -> Result<RenameResult, String> {
    let old = check_job_path(&path)?;
    let name = new_name.trim().trim_end_matches(".relax").trim();
    if name.is_empty() || name.chars().any(|c| r#"<>:"/\|?*"#.contains(c) || c.is_control()) {
        return Err(r#"A job name cannot be empty or contain any of these characters: < > : " / \ | ? *"#.into());
    }
    let new = old.with_file_name(format!("{name}.relax"));
    if new.exists()
        && !new
            .display()
            .to_string()
            .eq_ignore_ascii_case(&old.display().to_string())
    {
        return Err(format!("A job called \"{name}\" already exists in that folder."));
    }
    let had_schedule = runner::get_schedule(&old).map(|s| s.exists).unwrap_or(false);
    std::fs::rename(&old, &new).map_err(|e| format!("Cannot rename: {e}"))?;
    if had_schedule {
        let _ = runner::delete_schedule(&old);
    }
    Ok(RenameResult {
        path: new.display().to_string(),
        schedule_removed: had_schedule,
    })
}

/// Moves a job file to the Recycle Bin (so it can be restored) and removes its schedule.
#[tauri::command]
pub fn delete_job_file(path: String) -> Result<(), String> {
    let p = check_job_path(&path)?;
    let _ = runner::delete_schedule(&p);
    winfs::move_to_recycle_bin(&p)
        .or_else(|_| std::fs::remove_file(&p))
        .map_err(|e| e.to_string())
}
