//! Relax - Your data is safe!
//! Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
//! Website: https://vsa.edu.in/ · Email: softinquiry@vsa.edu.in

mod commands;
pub mod engine;
mod license;
mod runner;

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct AppInfo {
    name: String,
    version: String,
    developed_by: &'static str,
    organization: &'static str,
    website: &'static str,
    email: &'static str,
}

#[tauri::command]
fn app_info(app: tauri::AppHandle) -> AppInfo {
    let pkg = app.package_info();
    AppInfo {
        name: "Relax - Your data is safe!".into(),
        version: pkg.version.to_string(),
        developed_by: "Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar",
        organization: "Vidyasagar Academy",
        website: "https://vsa.edu.in/",
        email: "softinquiry@vsa.edu.in",
    }
}

/// Unattended run for Windows Task Scheduler: `Relax.exe --run "job.relax"`. Returns the exit code.
pub fn run_batch(job_path: &str) -> i32 {
    runner::run_batch(std::path::Path::new(job_path))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            app_info,
            license::get_license,
            license::buy_app,
            commands::compare_folders,
            commands::set_view,
            commands::get_rows,
            commands::set_direction,
            commands::start_sync,
            commands::answer_error,
            commands::set_paused,
            commands::cancel_operation,
            commands::reveal_path,
            commands::load_job_file,
            commands::save_job_file,
            commands::list_logs,
            commands::read_log,
            commands::open_logs_folder,
            commands::get_schedule,
            commands::create_schedule,
            commands::delete_schedule,
            commands::startup_job,
            commands::default_jobs_dir,
            commands::list_jobs,
            commands::rename_job_file,
            commands::delete_job_file,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Relax");
}
