//! Thin, safe wrappers around the Windows file APIs the executor needs:
//! native file copy with progress/cancel, Recycle Bin, free disk space and local time.

use std::ffi::c_void;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Component, Path, PathBuf, Prefix};
use windows_sys::Win32::Foundation::{HANDLE, SYSTEMTIME};
use windows_sys::Win32::Storage::FileSystem::{
    CopyFileExW, GetDiskFreeSpaceExW, GetDriveTypeW, COPY_FILE_FAIL_IF_EXISTS, PROGRESS_CANCEL, PROGRESS_CONTINUE,
};
use windows_sys::Win32::System::SystemInformation::GetLocalTime;
use windows_sys::Win32::UI::Shell::{
    SHFileOperationW, SHQueryRecycleBinW, FOF_ALLOWUNDO, FOF_NOCONFIRMATION, FOF_NOERRORUI, FOF_SILENT, FO_DELETE,
    SHFILEOPSTRUCTW, SHQUERYRBINFO,
};

const MAX_PATH: usize = 260;
const DRIVE_FIXED: u32 = 3;
/// Ask before deleting permanently if Windows cannot put an item into the Recycle Bin (e.g. it is too big).
const FOF_WANTNUKEWARNING: u32 = 0x4000;

fn wide(s: &std::ffi::OsStr) -> Vec<u16> {
    s.encode_wide().chain(std::iter::once(0)).collect()
}

/// `\\?\` form of an absolute path, so the Windows APIs accept paths longer than 260 characters.
pub fn long_path(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let s = abs.as_os_str().to_string_lossy().replace('/', "\\");
    if s.starts_with(r"\\?\") {
        PathBuf::from(s)
    } else if let Some(unc) = s.strip_prefix(r"\\") {
        PathBuf::from(format!(r"\\?\UNC\{unc}"))
    } else {
        PathBuf::from(format!(r"\\?\{s}"))
    }
}

/// Root of the drive a path is on, e.g. `F:\` or `\\server\share\`.
pub fn drive_root(path: &Path) -> Option<PathBuf> {
    let abs = std::path::absolute(path).ok()?;
    match abs.components().next()? {
        Component::Prefix(p) => match p.kind() {
            Prefix::Disk(d) | Prefix::VerbatimDisk(d) => Some(PathBuf::from(format!("{}:\\", d as char))),
            Prefix::UNC(server, share) | Prefix::VerbatimUNC(server, share) => Some(PathBuf::from(format!(
                r"\\{}\{}\",
                server.to_string_lossy(),
                share.to_string_lossy()
            ))),
            _ => None,
        },
        _ => None,
    }
}

/// Free bytes available to this user on the drive holding `path` (the folder itself may not exist yet).
pub fn free_space(path: &Path) -> Option<u64> {
    let root = drive_root(path)?;
    let w = wide(root.as_os_str());
    let mut free: u64 = 0;
    // SAFETY: `w` is a valid NUL-terminated wide string; the out-pointers are valid or null.
    let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, std::ptr::null_mut(), std::ptr::null_mut()) };
    (ok != 0).then_some(free)
}

/// Whether Windows keeps a Recycle Bin on the drive holding `path`.
/// USB flash drives, memory cards and network drives have none: deleting there is permanent.
pub fn has_recycle_bin(path: &Path) -> bool {
    let Some(root) = drive_root(path) else { return false };
    let w = wide(root.as_os_str());
    // SAFETY: valid NUL-terminated wide string.
    if unsafe { GetDriveTypeW(w.as_ptr()) } != DRIVE_FIXED {
        return false;
    }
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: valid string and a correctly sized out-struct.
    unsafe { SHQueryRecycleBinW(w.as_ptr(), &mut info) >= 0 }
}

/// Moves a file or (empty) folder to the Recycle Bin. Fails for paths the shell cannot handle (over 260 chars).
pub fn move_to_recycle_bin(path: &Path) -> io::Result<()> {
    let abs = std::path::absolute(path)?;
    if abs.as_os_str().len() >= MAX_PATH {
        return Err(io::Error::other("path too long for the Recycle Bin"));
    }
    // pFrom must be double-NUL terminated.
    let mut from = wide(abs.as_os_str());
    from.push(0);
    let mut op = SHFILEOPSTRUCTW {
        wFunc: FO_DELETE,
        pFrom: from.as_ptr(),
        fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI | FOF_WANTNUKEWARNING) as u16,
        ..Default::default()
    };
    // SAFETY: `op` is fully initialized and `from` outlives the call.
    let rc = unsafe { SHFileOperationW(&mut op) };
    if rc != 0 {
        return Err(io::Error::other(format!(
            "the Recycle Bin refused the item (code {rc:#x})"
        )));
    }
    if op.fAnyOperationsAborted != 0 {
        return Err(io::Error::other("moving to the Recycle Bin was cancelled"));
    }
    if abs.exists() {
        return Err(io::Error::other(
            "the item is still there after moving it to the Recycle Bin",
        ));
    }
    Ok(())
}

struct CopyContext<'a> {
    on_progress: &'a mut dyn FnMut(u64) -> bool,
    last: u64,
}

unsafe extern "system" fn copy_progress(
    _total: i64,
    transferred: i64,
    _stream_size: i64,
    _stream_transferred: i64,
    _stream_number: u32,
    _reason: u32,
    _src: HANDLE,
    _dst: HANDLE,
    data: *const c_void,
) -> u32 {
    // SAFETY: `data` is the `CopyContext` passed by `copy_file`, alive for the whole CopyFileExW call.
    let ctx = unsafe { &mut *(data as *mut CopyContext) };
    let transferred = transferred.max(0) as u64;
    let delta = transferred.saturating_sub(ctx.last);
    ctx.last = transferred;
    if (ctx.on_progress)(delta) {
        PROGRESS_CONTINUE
    } else {
        PROGRESS_CANCEL
    }
}

/// Copies a file with the native Windows copy engine (keeps the modification time, attributes and
/// alternate data streams). `dest` must not exist. `on_progress` gets newly copied bytes and returns
/// `false` to cancel; a cancelled copy leaves no partial file behind.
pub fn copy_file(src: &Path, dest: &Path, on_progress: &mut dyn FnMut(u64) -> bool) -> io::Result<()> {
    let s = wide(long_path(src).as_os_str());
    let d = wide(long_path(dest).as_os_str());
    let mut ctx = CopyContext { on_progress, last: 0 };
    // SAFETY: valid NUL-terminated strings; `ctx` outlives the call and is only used by `copy_progress`.
    let ok = unsafe {
        CopyFileExW(
            s.as_ptr(),
            d.as_ptr(),
            Some(copy_progress),
            &mut ctx as *mut CopyContext as *const c_void,
            std::ptr::null_mut(),
            COPY_FILE_FAIL_IF_EXISTS,
        )
    };
    if ok == 0 {
        let err = io::Error::last_os_error();
        let _ = std::fs::remove_file(dest);
        return Err(err);
    }
    Ok(())
}

/// Local date and time as `YYYY-MM-DD HHMMSS`, used to name version folders.
pub fn local_timestamp() -> String {
    let mut t = SYSTEMTIME::default();
    // SAFETY: valid out-pointer.
    unsafe { GetLocalTime(&mut t) };
    format!(
        "{:04}-{:02}-{:02} {:02}{:02}{:02}",
        t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_path_forms() {
        assert_eq!(long_path(Path::new(r"C:\a\b")), PathBuf::from(r"\\?\C:\a\b"));
        assert_eq!(
            long_path(Path::new(r"\\srv\share\x")),
            PathBuf::from(r"\\?\UNC\srv\share\x")
        );
        assert_eq!(long_path(Path::new(r"\\?\C:\a")), PathBuf::from(r"\\?\C:\a"));
    }

    #[test]
    fn drive_roots() {
        assert_eq!(drive_root(Path::new(r"F:\Backup\x")), Some(PathBuf::from(r"F:\")));
        assert_eq!(
            drive_root(Path::new(r"\\srv\share\x")),
            Some(PathBuf::from(r"\\srv\share\"))
        );
    }

    #[test]
    fn free_space_of_temp_drive() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(free_space(&tmp.path().join("not-yet-created")).unwrap() > 0);
    }

    #[test]
    fn native_copy_keeps_time_and_reports_progress() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.bin");
        let dst = tmp.path().join("dst.bin");
        std::fs::write(&src, vec![1u8; 3_000_000]).unwrap();
        let t = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_700_000_000);
        std::fs::File::options()
            .write(true)
            .open(&src)
            .unwrap()
            .set_modified(t)
            .unwrap();
        let mut total = 0;
        copy_file(&src, &dst, &mut |n| {
            total += n;
            true
        })
        .unwrap();
        assert_eq!(total, 3_000_000);
        assert_eq!(std::fs::read(&dst).unwrap().len(), 3_000_000);
        assert_eq!(std::fs::metadata(&dst).unwrap().modified().unwrap(), t);
        // Destination exists now: a second copy must fail instead of overwriting.
        assert!(copy_file(&src, &dst, &mut |_| true).is_err());
    }

    #[test]
    fn cancelled_copy_leaves_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src.bin");
        let dst = tmp.path().join("dst.bin");
        std::fs::write(&src, vec![1u8; 8_000_000]).unwrap();
        assert!(copy_file(&src, &dst, &mut |_| false).is_err());
        assert!(!dst.exists());
    }

    #[test]
    fn timestamp_format() {
        let s = local_timestamp();
        assert_eq!(s.len(), 17, "{s}");
        assert_eq!(&s[4..5], "-");
    }
}
