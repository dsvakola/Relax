// Relax - Your data is safe!
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

// Prevents an extra console window on Windows in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // Scheduled / unattended run: Relax.exe --run "C:\path\job.relax"
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a.eq_ignore_ascii_case("--run")) {
        let code = match args.get(i + 1) {
            Some(job) => relax_lib::run_batch(job),
            None => 2,
        };
        std::process::exit(code);
    }
    relax_lib::run()
}
