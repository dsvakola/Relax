# Relax 2.0: Professor's review notes (19 Oct 2026) — ALL DONE in Round 2 (release built 19 Oct 13:32)

Noted only; nothing changed yet. To be done when the Professor gives the go-ahead.

## 1. Installer quality (looks blurry, low quality)
- Sidebar and header pictures look soft and pixelated on high-DPI screens (they are drawn at 100% size and stretched by Windows).
- Text in the installer windows should be crisp and clearly readable.
- The bottom "Copyright … Prof. Yash" branding line is cut off; shorten it or make it fit.
- Product name "Relax - Your data is safe!" repeated in installer sentences looks clumsy ("Setup will guide you through the installation of Relax - Your data is safe!.").

## 2. Window resizing (unprofessional at smaller sizes)
- At narrower widths the toolbar is cut off (Synchronize, 100% Offline, Help and About disappear off the right edge).
- Check every size down to the minimum window size; nothing may be cut off or overlap.

## 3. Menu bar: File, Edit, Tools, Help
- Add a proper menu bar with relevant sub-menus (e.g. File: New/Open/Save/Save as/Recent jobs/Exit; Edit: folder pairs, select all, change actions; Tools: Compare, Synchronize, Filter, Settings, Schedule, Logs; Help: User guide, Shortcuts, Website, Contact, About).

## 4. Progress while comparing (barely visible)
- "Scanning folders… N items found" is only in the bottom-left status bar.
- Show it in the middle of the grid area, larger (about 11 pt), so the user clearly sees work is going on.

## 5. Saved jobs list in a left pane (like FreeFileSync)
- Left side panel listing saved .relax jobs; click to load, double-click to compare.
- Overall window look close to FreeFileSync's (installed and portable), but our own style, not a copy.

## 6. Grid: files and action symbols
- Professor sees folders but no files. Cause: identical files are hidden by default ("=" view button off); only the new folders and their parents show. Make this obvious (e.g. show a hint "12,139 identical items hidden – show them"), or consider showing files by default.
- Make the action symbols bolder and larger. Professor asked: "=" green, "+→" red.
  (Point to discuss: red usually means "delete" in sync tools; suggest red only for delete, a strong colour for copy.)

## Still to come from the Professor
- Results of testing the installed and portable versions with real backups.
