//! Microsoft Store licence and 7-day free trial (Windows.Services.Store).
//! Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
//!
//! Rules:
//! - Only the Microsoft Store (MSIX) package is licensed. The installer, the portable EXE and `pnpm tauri dev`
//!   are not packaged, so they report `free` and show nothing.
//! - Windows answers from its own licence cache, so this also works offline. The last good answer is saved in
//!   the app data folder as a fallback; if nothing is known at all, the result is `unknown` and Relax stays usable.
//! - This module never reads, locks, changes or deletes the user's files, jobs or backups.

use crate::runner;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

/// Store ID of "Relax - Your data is safe!" in Partner Center.
const STORE_ID: &str = "9N4BPLF2TD95";
/// Shown on the Buy screen. The Store itself shows and charges the real price.
const PRICE_TEXT: &str = "Rs. 299, one-time purchase";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LicenseInfo {
    /// `free` (not a Store install), `trial`, `licensed`, `expired` or `unknown`.
    pub state: &'static str,
    /// Whole days left; only for `trial`.
    pub days_left: Option<u32>,
    pub price: &'static str,
}

impl LicenseInfo {
    fn new(state: &'static str, days_left: Option<u32>) -> Self {
        Self {
            state,
            days_left,
            price: PRICE_TEXT,
        }
    }
}

/// What is remembered between runs, for when the Store cannot be asked.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cache {
    state: String,
    /// Unix seconds when the trial ends (trial only).
    trial_ends_at: Option<i64>,
}

fn cache_file() -> PathBuf {
    runner::data_dir().join("license.json")
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn save_cache(c: &Cache) {
    let file = cache_file();
    let Some(dir) = file.parent() else { return };
    let Ok(text) = serde_json::to_string(c) else { return };
    // Atomic write: temporary file, then rename.
    let tmp = file.with_extension("json.tmp");
    if fs::create_dir_all(dir).is_ok() && fs::write(&tmp, text).is_ok() {
        let _ = fs::rename(&tmp, &file);
    }
}

fn from_cache() -> LicenseInfo {
    let Ok(text) = fs::read_to_string(cache_file()) else {
        return LicenseInfo::new("unknown", None);
    };
    let Ok(c) = serde_json::from_str::<Cache>(&text) else {
        return LicenseInfo::new("unknown", None);
    };
    match c.state.as_str() {
        "licensed" => LicenseInfo::new("licensed", None),
        "expired" => LicenseInfo::new("expired", None),
        "trial" => match c.trial_ends_at {
            Some(end) if end > now_secs() => LicenseInfo::new("trial", Some(days_from_secs(end - now_secs()))),
            Some(_) => LicenseInfo::new("expired", None),
            None => LicenseInfo::new("unknown", None),
        },
        _ => LicenseInfo::new("unknown", None),
    }
}

/// Whole days, rounded up (a trial with 6 days and 3 hours left shows "7 days left").
fn days_from_secs(secs: i64) -> u32 {
    ((secs.max(0) + 86_399) / 86_400) as u32
}

/// True when running from the Microsoft Store (MSIX) package.
fn is_packaged() -> bool {
    use windows_sys::Win32::Storage::Packaging::Appx::GetCurrentPackageFullName;
    const APPMODEL_ERROR_NO_PACKAGE: u32 = 15700;
    let mut len: u32 = 0;
    // SAFETY: a zero-length query with a null buffer is the documented way to ask whether a package exists.
    let rc = unsafe { GetCurrentPackageFullName(&mut len, std::ptr::null_mut()) };
    rc != APPMODEL_ERROR_NO_PACKAGE
}

/// For testing the screens in a debug build: `RELAX_LICENSE_SIM=trial:5`, `expired`, `licensed` or `unknown`.
#[cfg(debug_assertions)]
fn simulated() -> Option<LicenseInfo> {
    let v = std::env::var("RELAX_LICENSE_SIM").ok()?;
    let (kind, arg) = v.split_once(':').unwrap_or((v.as_str(), ""));
    Some(match kind {
        "trial" => LicenseInfo::new("trial", Some(arg.parse().unwrap_or(7))),
        "expired" => LicenseInfo::new("expired", None),
        "licensed" => LicenseInfo::new("licensed", None),
        "unknown" => LicenseInfo::new("unknown", None),
        _ => LicenseInfo::new("free", None),
    })
}
#[cfg(not(debug_assertions))]
fn simulated() -> Option<LicenseInfo> {
    None
}

/// Asks Windows (Microsoft Store) for the app licence. Must run on its own thread.
fn ask_store() -> Result<LicenseInfo, String> {
    use windows::Services::Store::StoreContext;
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
    // SAFETY: plain COM initialisation of this fresh thread; "already initialised" is not an error here.
    let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    let ctx = StoreContext::GetDefault().map_err(|e| e.message())?;
    let lic = ctx
        .GetAppLicenseAsync()
        .map_err(|e| e.message())?
        .join()
        .map_err(|e| e.message())?;
    let active = lic.IsActive().map_err(|e| e.message())?;
    let trial = lic.IsTrial().map_err(|e| e.message())?;
    if active && trial {
        // TimeSpan counts 100-nanosecond ticks.
        let secs = lic.TrialTimeRemaining().map_err(|e| e.message())?.Duration / 10_000_000;
        save_cache(&Cache {
            state: "trial".into(),
            trial_ends_at: Some(now_secs() + secs),
        });
        if secs <= 0 {
            return Ok(LicenseInfo::new("expired", None));
        }
        return Ok(LicenseInfo::new("trial", Some(days_from_secs(secs))));
    }
    if active {
        save_cache(&Cache {
            state: "licensed".into(),
            trial_ends_at: None,
        });
        return Ok(LicenseInfo::new("licensed", None));
    }
    save_cache(&Cache {
        state: "expired".into(),
        trial_ends_at: None,
    });
    Ok(LicenseInfo::new("expired", None))
}

/// Current licence state. Never fails: when the Store cannot be asked, the saved answer (or `unknown`) is used.
pub fn check() -> LicenseInfo {
    if let Some(s) = simulated() {
        return s;
    }
    if !is_packaged() {
        return LicenseInfo::new("free", None);
    }
    match std::thread::spawn(ask_store).join() {
        Ok(Ok(info)) => info,
        _ => from_cache(),
    }
}

/// Opens the Microsoft Store purchase dialog on top of the Relax window.
fn buy_blocking(hwnd: isize) -> Result<LicenseInfo, String> {
    if simulated().is_some() {
        return Err("Purchase is not available in a test run.".into());
    }
    if !is_packaged() {
        return Err("Buying is only possible in the Microsoft Store version of Relax.".into());
    }
    let result = std::thread::spawn(move || -> Result<(), String> {
        use windows::core::{Interface, HSTRING};
        use windows::Services::Store::{StoreContext, StorePurchaseStatus};
        use windows::Win32::Foundation::HWND;
        use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};
        use windows::Win32::UI::Shell::IInitializeWithWindow;
        // SAFETY: plain COM initialisation of this fresh thread.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let ctx = StoreContext::GetDefault().map_err(|e| e.message())?;
        // A desktop app must tell the Store which window the purchase dialog belongs to.
        let init: IInitializeWithWindow = ctx.cast().map_err(|e| e.message())?;
        // SAFETY: `hwnd` is the handle of Relax's own main window, passed in by the caller.
        unsafe { init.Initialize(HWND(hwnd as *mut _)) }.map_err(|e| e.message())?;
        let res = ctx
            .RequestPurchaseAsync(&HSTRING::from(STORE_ID))
            .map_err(|e| e.message())?
            .join()
            .map_err(|e| e.message())?;
        match res.Status().map_err(|e| e.message())? {
            s if s == StorePurchaseStatus::Succeeded || s == StorePurchaseStatus::AlreadyPurchased => Ok(()),
            s if s == StorePurchaseStatus::NotPurchased => Ok(()), // closed the dialog: nothing to report
            s if s == StorePurchaseStatus::NetworkError => {
                Err("The Microsoft Store could not be reached. Please check your internet connection.".into())
            }
            _ => Err("The Microsoft Store could not complete the purchase. Please try again.".into()),
        }
    })
    .join();
    match result {
        Ok(Ok(())) => Ok(check()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("The purchase window could not be opened.".into()),
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub async fn get_license() -> LicenseInfo {
    tauri::async_runtime::spawn_blocking(check)
        .await
        .unwrap_or_else(|_| LicenseInfo::new("unknown", None))
}

#[tauri::command]
pub async fn buy_app(window: tauri::WebviewWindow) -> Result<LicenseInfo, String> {
    let hwnd = window.hwnd().map_err(|e| e.to_string())?.0 as isize;
    tauri::async_runtime::spawn_blocking(move || buy_blocking(hwnd))
        .await
        .map_err(|_| "The purchase could not be started.".to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_round_up() {
        assert_eq!(days_from_secs(0), 0);
        assert_eq!(days_from_secs(1), 1);
        assert_eq!(days_from_secs(86_400), 1);
        assert_eq!(days_from_secs(86_401), 2);
        assert_eq!(days_from_secs(6 * 86_400 + 3 * 3600), 7);
        assert_eq!(days_from_secs(-5), 0);
    }

    #[test]
    fn unpackaged_test_process_is_free() {
        // `cargo test` does not run from a Store package.
        assert!(!is_packaged());
    }
}
