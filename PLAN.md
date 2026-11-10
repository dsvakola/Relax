# Relax 2.0 — Fresh Start Plan (FreeFileSync-class backup & sync)

Built with Tauri 2 + Rust (WebView2). Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy. https://vsa.edu.in/ · softinquiry@vsa.edu.in

## Guiding rule: reliability first
- Nothing is ever deleted or overwritten without a preview the user has seen.
- Deleted/overwritten files go to the **Recycle Bin** (default) or a **Versioning folder**, never straight to oblivion unless chosen.
- Every copy is written to a temporary name and renamed only when complete and verified (no half-written files after a power cut or unplugged USB).
- The sync engine is written and tested in Rust first, with automated tests on real temporary folders, before any UI is connected.

## Features for version 2.0 (FreeFileSync parity, core)
| Area | Features |
|---|---|
| Folder pairs | Multiple left/right pairs per job; drive picked by **volume name** so USB drives work even when the letter changes |
| Compare | By file time & size (default) · by content (byte-by-byte) · by size only |
| Sync variants | **Two-way** (detects deletions and conflicts using a small sync database) · **Mirror** (exact backup) · **Update** (copy new/changed, never delete) · **Custom** (choose action per category) |
| Preview grid | Side-by-side file list like FreeFileSync, action icon per row, change an action by right-click, filter by category, totals of what will be copied/deleted |
| Deletion handling | Recycle Bin · Versioning folder (timestamped) · Permanent |
| Filters | Include/exclude with wildcards (`*.tmp`, `\node_modules\`, `~$*`), min/max size, modified-since |
| File safety | Temp-then-rename copy · preserve timestamps & attributes · long paths over 260 chars · optional verify after copy · skip files in use with a clear message |
| Running | Progress with speed and time left · pause · cancel · errors shown with Retry / Ignore / Ignore all |
| Jobs | Save/load jobs (`.relax` files) · batch jobs run from command line for **Windows Task Scheduler** |
| Logs | Log of every sync saved automatically, viewable in the app |
| Packaging | NSIS installer with VSA branding + portable EXE |

## Later versions (2.1+)
- RealTimeSync (watch folders and sync automatically on change)
- Detect moved/renamed files (avoid re-copying)
- Copy locked files via Volume Shadow Copy (needs admin)
- Built-in scheduler, e-mail-free desktop notifications

## Build steps (one at a time, with your go-ahead)
1. Scaffold the Tauri project, window, icons, VSA branding, About box.
2. Rust engine: folder scanning and comparison, with automated tests.
3. Rust engine: sync plan for Two-way / Mirror / Update / Custom, with tests.
4. Rust engine: safe execution (temp-then-rename, Recycle Bin, versioning, verify), pause/cancel, errors.
5. UI: main window with folder pairs, Compare and Synchronize buttons, preview grid.
6. Settings dialogs: compare, filter, sync; save/load jobs.
7. Batch jobs + command line, logs.
8. Installer + portable EXE, full end-to-end testing, size/memory measurement.
