# Relax - Your data is safe!

**Reliable, 100% offline backup and folder synchronization for Windows (version 2.1.0)**

Developed by **Prof. Dattaraj S. Vidyasagar** & **Prof. Yash D. Vidyasagar**, for **Vidyasagar Academy**.
- Website: [https://vsa.edu.in/](https://vsa.edu.in/)
- Email: [softinquiry@vsa.edu.in](mailto:softinquiry@vsa.edu.in)
- Source and privacy policy: [github.com/dsvakola/relax](https://github.com/dsvakola/relax) · [PRIVACY.md](https://github.com/dsvakola/relax/blob/main/PRIVACY.md)

Relax compares two folders, shows every difference in a preview, and changes nothing until you click **Synchronize**. It never connects to the internet.

## Features

- **Folder pairs:** one or more left (source) and right (target) folders per job. To back up to two drives, add a second pair with the same left folder.
- **Compare** by file time and size (2-second tolerance for USB drives), by content (byte by byte), or by size.
- **Sync modes:**
  - **Mirror:** the right side becomes an exact copy of the left side.
  - **Update:** incremental backup; only new and changed files are copied, and nothing is deleted.
  - **Two-way:** changes on both sides are copied across, and Relax remembers the last sync so it can tell deletions from new files.
  - **Custom:** your own direction for each kind of difference.
- **Preview grid** for hundreds of thousands of files, with view filters and a right-click menu to change any action.
- **Safety first:**
  - Each copy is written to a temporary file and renamed only when complete.
  - Replaced and deleted files go to the Recycle Bin, or to a `Relax-Versions` folder on drives without one, or to a versions folder you choose.
  - Nothing is deleted when the source folder is missing or empty.
  - Left and right can never be the same folder or inside each other.
  - Free space is checked before starting.
  - A file changed after the comparison is never overwritten.
- **Pause, cancel, Retry / Ignore / Ignore all** while synchronizing.
- **Filters** with FreeFileSync-style wildcards (`*.tmp`, `node_modules`, `\Downloads\`).
- **Jobs** saved as `.relax` files. Double-click a job file to open it.
- **Scheduled runs** through Windows Task Scheduler (daily, weekly or at sign-in), without a window.
- **Microsoft Store version:** Rs. 299 one-time purchase with a 7-day free trial (a "Trial: N days left" badge and a Buy button; after the trial a Buy Relax screen). The licence check is done by Windows and the Store; your files, jobs and backups are never touched or locked. Other builds (installer, portable, development) show no trial.
- **Logs** of every sync. Open them from Tools > Logs (the "open folder" button always shows the right place). The installer and portable versions keep them in `%APPDATA%\in.edu.vsa.relax\logs`; the Microsoft Store version keeps them (and settings and the sync database) in its own protected app folder, `%LOCALAPPDATA%\Packages\<package>\LocalCache\Roaming\in.edu.vsa.relax\logs`, which Windows removes when the app is uninstalled.

## Using Relax

1. Choose the left and right folders (Browse, or drag a folder from Explorer).
2. Choose the mode: **Mirror**, **Update**, **Two-way** or **Custom** (⚙ for all settings).
3. Click **Compare** (F5) and check the list.
4. Click **Synchronize** (F9).

To run a job without the window: `Relax.exe --run "D:\Jobs\My backup.relax"`. The exit code is 0 for OK, 1 for finished with problems, and 2 for failed.

## Building from source

Requirements: Windows 10/11, [Rust](https://rustup.rs) (MSVC), Visual Studio Build Tools (C++), Node.js 18+ and [pnpm](https://pnpm.io) (pnpm only; npm and yarn are not used). The WebView2 runtime is built into Windows 10/11.

Double-click **`Build-Relax.bat`**. It prepares a static C runtime (so Relax needs no VC++ runtime on the PC), installs the build tools with pnpm, runs the engine tests, and puts the results in `Release\`:
- `Relax-Setup-2.1.0.exe`: the installer (per user, no administrator rights needed).
- `Relax-Portable-2.1.0.exe`: a single file that runs from anywhere, including a USB stick.

Microsoft Store package: run `BUILD-STORE-PACKAGE.bat` (makes the unsigned `Release\Store\Relax_<version>_x64.msix` to upload in Partner Center), and `TEST-STORE-PACKAGE.bat` as administrator to try a test-signed copy and run the Windows App Certification Kit. Put the Identity Name from Partner Center in `store\identity.txt` first. The Store version asks for no administrator rights (capability `runFullTrust` only).

For development: `pnpm install`, then `pnpm tauri dev`.

## Technology

- **Tauri 2** with the WebView2 engine built into Windows (no bundled browser).
- **Rust** backend: scanning, comparing, planning and copying (`src-tauri/src/engine`), with 70+ automated tests (`cargo test`).
- **Vanilla TypeScript** frontend built with Vite (`src/`).

## Copyright

Copyright © 2026 Vidyasagar Academy. All rights reserved. See `NOTICE.txt`.
