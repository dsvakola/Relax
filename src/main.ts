// Relax - Your data is safe!
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
// Website: https://vsa.edu.in/ · Email: softinquiry@vsa.edu.in

import './styles.css';
import './screens.css';
import './layout.css';
import { invoke } from '@tauri-apps/api/core';
import { getCurrentWindow } from '@tauri-apps/api/window';
import { getCurrentWebview } from '@tauri-apps/api/webview';
import {
  api,
  type CompareVariant,
  type Direction,
  type ErrorDecision,
  type GridRow,
  type Summary,
  type SyncEvent,
  type SyncOutcome,
  type SyncVariant,
  type ViewFilter,
} from './api';
import { actionVerb, esc, formatBytes, formatCount, formatDuration } from './format';
import { VirtualGrid } from './grid';
import { openLogs, openSchedule, refreshScheduleDot } from './extras';
import { initLicense } from './license';
import { ask, initJobsPane, initMenus, refreshJobs, togglePane, type Commands } from './chrome';
import {
  buildJob,
  execOptions,
  filterIsDefault,
  newJob,
  openFilter,
  openJob,
  openSettings,
  prefs,
  saveJob,
  saveJobAs,
  savePrefs,
  updateTitle,
  jobName,
  loadJobFromPath,
  knownJobs,
  forgetJob,
  jobRenamed,
  isDirty,
  type Prefs,
} from './settings';

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing element #${id}`);
  return node as T;
}

// ---------------------------------------------------------------------------
// Simple dialogs
// ---------------------------------------------------------------------------

function showModal(id: string): void {
  el(id).hidden = false;
}
function hideModal(id: string): void {
  el(id).hidden = true;
}

function message(title: string, text: string, logo = false): Promise<void> {
  el('msgTitle').textContent = title;
  el('msgLogo').hidden = !logo;
  el('msgText').textContent = text;
  showModal('msgModal');
  el('btnMsgOk').focus();
  return new Promise((resolve) => {
    el('btnMsgOk').onclick = () => {
      hideModal('msgModal');
      resolve();
    };
  });
}

function setStatus(text: string): void {
  el('statusLeft').textContent = text;
}

// ---------------------------------------------------------------------------
// Folder pairs
// ---------------------------------------------------------------------------

const pairRows = el('pairRows');

function renderPairs(): void {
  pairRows.innerHTML = '';
  prefs.pairs.forEach((pair, index) => {
    const row = document.createElement('div');
    row.className = 'pair-row';
    row.innerHTML = `
      <div class="side">
        <input class="path-input" data-side="left" type="text" placeholder="Drag a folder here or click Browse" spellcheck="false">
        <button class="btn btn-browse" data-side="left">Browse</button>
      </div>
      <div class="pair-mid">
        <button class="btn-round" data-act="swap" title="Swap left and right">⇄</button>
        ${prefs.pairs.length > 1 ? '<button class="btn-round btn-remove" data-act="remove" title="Remove this folder pair">✕</button>' : ''}
      </div>
      <div class="side">
        <input class="path-input" data-side="right" type="text" placeholder="Drag a folder here or click Browse" spellcheck="false">
        <button class="btn btn-browse" data-side="right">Browse</button>
      </div>`;
    const inputs = row.querySelectorAll<HTMLInputElement>('.path-input');
    inputs[0].value = pair.left;
    inputs[1].value = pair.right;
    inputs.forEach((inp) =>
      inp.addEventListener('input', () => {
        pair[inp.dataset.side as 'left' | 'right'] = inp.value;
        savePrefs();
        markStale();
      }),
    );
    row.querySelectorAll<HTMLButtonElement>('.btn-browse').forEach((btn) =>
      btn.addEventListener('click', async () => {
        const side = btn.dataset.side as 'left' | 'right';
        const picked = await api.pickFolder(
          side === 'left' ? 'Select left (source) folder' : 'Select right (target) folder',
          pair[side] || undefined,
        );
        if (picked) {
          pair[side] = picked;
          savePrefs();
          renderPairs();
          markStale();
        }
      }),
    );
    row.querySelector('[data-act="swap"]')!.addEventListener('click', () => {
      [pair.left, pair.right] = [pair.right, pair.left];
      savePrefs();
      renderPairs();
      markStale();
    });
    row.querySelector('[data-act="remove"]')?.addEventListener('click', () => {
      prefs.pairs.splice(index, 1);
      savePrefs();
      renderPairs();
      markStale();
    });
    pairRows.appendChild(row);
  });
}

el('btnAddPair').addEventListener('click', () => {
  prefs.pairs.push({ left: '', right: '' });
  savePrefs();
  renderPairs();
});

// Drag and drop a folder from Explorer onto a path box.
let dropTarget: HTMLInputElement | null = null;
function inputAt(x: number, y: number): HTMLInputElement | null {
  const ratio = window.devicePixelRatio || 1;
  const node = document.elementFromPoint(x / ratio, y / ratio);
  return (node?.closest('.side')?.querySelector('.path-input') as HTMLInputElement | null) ?? null;
}
void getCurrentWebview().onDragDropEvent((event) => {
  const p = event.payload;
  dropTarget?.classList.remove('drop-hover');
  if (p.type === 'over' || p.type === 'enter') {
    dropTarget = inputAt(p.position.x, p.position.y);
    dropTarget?.classList.add('drop-hover');
  } else if (p.type === 'drop') {
    const input = inputAt(p.position.x, p.position.y);
    if (input && p.paths.length > 0) {
      input.value = p.paths[0];
      input.dispatchEvent(new Event('input'));
    }
    dropTarget = null;
  } else {
    dropTarget = null;
  }
});

// ---------------------------------------------------------------------------
// Toolbar options, settings, filter and job files
// ---------------------------------------------------------------------------

const selCompare = el<HTMLSelectElement>('selCompare');
const selSync = el<HTMLSelectElement>('selSync');

/** Shows the current settings in the toolbar and folder boxes. */
function showPrefs(): void {
  selCompare.value = prefs.compareVariant;
  selSync.value = prefs.syncVariant;
  el('filterDot').hidden = filterIsDefault();
  el('btnFilter').title = filterIsDefault()
    ? 'Include / exclude filter'
    : `Filter active
Include: ${prefs.include.join(', ')}
Exclude: ${prefs.exclude.join(', ')}`;
  renderPairs();
}

selCompare.addEventListener('change', () => {
  prefs.compareVariant = selCompare.value as CompareVariant;
  savePrefs();
  markStale();
});
selSync.addEventListener('change', () => {
  prefs.syncVariant = selSync.value as SyncVariant;
  savePrefs();
  // The plan depends on the sync mode, so compare again right away if results are shown.
  if (summary) void runCompare();
});

/** Settings changed: show them, and compare again if results were on screen. */
function settingsChanged(): void {
  const hadResults = summary !== null;
  showPrefs();
  if (hadResults) void runCompare();
}

el('btnSettings').addEventListener('click', () => {
  if (!syncing && !comparing) openSettings(settingsChanged);
});
el('btnFilter').addEventListener('click', () => {
  if (!syncing && !comparing) openFilter(settingsChanged);
});

async function jobAction(action: () => Promise<string | null>, done: string): Promise<void> {
  if (syncing || comparing) return;
  try {
    const path = await action();
    void refreshScheduleDot();
    void refreshJobs();
    if (path) setStatus(`${done}: ${path}`);
  } catch (e) {
    await message('Job file', String(e instanceof Error ? e.message : e));
  }
}

/** Before leaving the current job: offer to save unsaved changes. Returns false to stay. */
async function confirmLeaveJob(): Promise<boolean> {
  if (!isDirty()) return true;
  const r = await ask({
    title: 'Save changes?',
    text: `The job "${jobName() || 'New job'}" has changes that are not saved yet.`,
    okLabel: 'Save',
    noLabel: "Don't save",
  });
  if (r.button === 'cancel') return false;
  if (r.button === 'ok') {
    try {
      return (await saveJob()) !== null;
    } catch (e) {
      await message('Job file', String(e instanceof Error ? e.message : e));
      return false;
    }
  }
  return true;
}

function cmdNewJob(): void {
  if (syncing || comparing) return;
  void confirmLeaveJob().then((go) => {
    if (!go) return;
    newJob();
    markStale();
    showPrefs();
    void refreshScheduleDot();
    void refreshJobs();
    setStatus('New job');
  });
}

function cmdOpenJob(): void {
  if (syncing || comparing) return;
  void confirmLeaveJob().then((go) => {
    if (!go) return;
    void jobAction(async () => {
      const p = await openJob();
      if (p) {
        markStale();
        showPrefs();
      }
      return p;
    }, 'Opened');
  });
}

/** Opens a job from the saved-jobs pane (and compares it on double-click). */
async function openJobFromPane(path: string, thenCompare: boolean): Promise<void> {
  if (syncing || comparing) return;
  const same = prefs.jobPath.toLowerCase() === path.toLowerCase();
  if (!same) {
    if (!(await confirmLeaveJob())) return;
    try {
      await loadJobFromPath(path);
    } catch (e) {
      await message('Job file', String(e instanceof Error ? e.message : e));
      return;
    }
    markStale();
    showPrefs();
    void refreshScheduleDot();
    void refreshJobs();
    setStatus(`Opened: ${path}`);
  }
  if (thenCompare) void runCompare();
}

const cmdSaveJob = () => void jobAction(saveJob, 'Saved');
const cmdSaveJobAs = () => void jobAction(saveJobAs, 'Saved');
const cmdSchedule = () => {
  if (!syncing && !comparing) void openSchedule(message).then(refreshScheduleDot);
};

el('btnSchedule').addEventListener('click', cmdSchedule);
el('btnLogs').addEventListener('click', () => void openLogs());
el('btnJobNew').addEventListener('click', cmdNewJob);
el('btnJobOpen').addEventListener('click', cmdOpenJob);
el('btnJobSave').addEventListener('click', (e) => (e.shiftKey ? cmdSaveJobAs() : cmdSaveJob()));

// ---------------------------------------------------------------------------
// Compare and the preview grid
// ---------------------------------------------------------------------------

const grid = new VirtualGrid(el('gridBody'));
let summary: Summary | null = null;
let comparing = false;
let syncing = false;
let view: ViewFilter = {
  showEqual: false,
  showSkipped: true,
  showCreate: true,
  showUpdate: true,
  showDelete: true,
  showConflict: true,
};

/** Folder settings changed: the shown results no longer match. */
function markStale(): void {
  if (!summary) return;
  summary = null;
  grid.clear();
  el<HTMLButtonElement>('btnSync').disabled = true;
  el('viewFilters').hidden = true;
  el('notices').hidden = true;
  showEmpty('Settings changed', 'Click Compare to see the differences again.');
  setStatus('Ready');
}

function showEmpty(title: string, html: string): void {
  el('emptyTitle').textContent = title;
  el('emptyText').innerHTML = html;
  el('gridEmpty').hidden = false;
}

async function runCompare(): Promise<void> {
  if (syncing) return;
  if (comparing) {
    await api.cancel();
    return;
  }
  const pairs = prefs.pairs
    .map((p) => ({ left: p.left.trim(), right: p.right.trim() }))
    .filter((p) => p.left || p.right);
  if (pairs.length === 0) {
    await message('Choose your folders', 'Please choose a left (source) and a right (target) folder first.');
    return;
  }
  if (pairs.some((p) => !p.left || !p.right)) {
    await message('Folder missing', 'Every folder pair needs both a left and a right folder.');
    return;
  }

  comparing = true;
  el('btnCompareText').textContent = 'Cancel';
  el('btnCompare').classList.add('tb-cancel');
  el<HTMLButtonElement>('btnSync').disabled = true;
  setStatus('Scanning folders…');
  showBusy('Scanning folders…', 'Starting…');
  const started = performance.now();
  try {
    summary = await api.compare(buildJob(pairs), (p) => {
      const secs = Math.round((performance.now() - started) / 1000);
      if (p.phase === 'scanning') {
        const txt = `${formatCount(p.itemsFound)} files and folders found`;
        showBusy(pairs.length > 1 ? `Scanning folder pair ${p.pairIndex + 1} of ${pairs.length}…` : 'Scanning folders…', `${txt} · ${secs} s`);
        setStatus(`Scanning folders… ${txt}`);
      } else {
        const txt = `${formatBytes(p.bytesDone)} of ${formatBytes(p.bytesTotal)}`;
        showBusy('Comparing file contents…', txt, p.bytesTotal ? p.bytesDone / p.bytesTotal : 0);
        setStatus(`Comparing file contents… ${txt}`);
      }
    });
    applySummary(false);
    const files = summary.pairs.reduce((a, p) => a + p.compareStats.leftFiles + p.compareStats.rightFiles, 0);
    setStatus(`Compared ${formatCount(files)} files in ${Math.max(0.1, (performance.now() - started) / 1000).toFixed(1)} s`);
  } catch (e) {
    summary = null;
    grid.clear();
    const msg = String(e);
    if (msg.includes('cancelled')) {
      setStatus('Comparison cancelled');
    } else {
      setStatus('Comparison failed');
      await message('Cannot compare', msg);
    }
  } finally {
    comparing = false;
    hideBusy();
    el('btnCompareText').textContent = 'Compare';
    el('btnCompare').classList.remove('tb-cancel');
  }
}

/** Large progress panel in the middle of the list while comparing. */
function showBusy(title: string, detail: string, fraction?: number): void {
  el('busy').hidden = false;
  el('busyTitle').textContent = title;
  el('busyDetail').textContent = detail;
  el('busyProgress').hidden = fraction === undefined;
  if (fraction !== undefined) el('busyFill').style.width = `${Math.min(100, fraction * 100).toFixed(1)}%`;
}

function hideBusy(): void {
  el('busy').hidden = true;
}

el('btnBusyCancel').addEventListener('click', () => {
  el('busyTitle').textContent = 'Cancelling…';
  void api.cancel();
});

/** Keeps the grid's column headers exactly above the columns (the list has a scrollbar, the header has not). */
function alignGridHeader(): void {
  const body = el('gridBody');
  el('gridHead').style.paddingRight = `${body.offsetWidth - body.clientWidth}px`;
}
window.addEventListener('resize', alignGridHeader);
new ResizeObserver(alignGridHeader).observe(el('gridBody'));

function totals(s: Summary) {
  const t = { create: 0, update: 0, del: 0, conflict: 0, equal: 0, bytes: 0 };
  for (const p of s.pairs) {
    for (const side of [p.stats.left, p.stats.right]) {
      t.create += side.createFiles + side.createFolders;
      t.update += side.updateFiles;
      t.del += side.deleteFiles + side.deleteFolders;
      t.bytes += side.bytesToWrite;
    }
    t.conflict += p.stats.conflicts;
    t.equal += p.compareStats.equal;
  }
  return t;
}

function applySummary(keepScroll: boolean): void {
  if (!summary) return;
  const s = summary;
  const t = totals(s);
  el('cntCreate').textContent = formatCount(t.create);
  el('cntUpdate').textContent = formatCount(t.update);
  el('cntDelete').textContent = formatCount(t.del);
  el('cntConflict').textContent = formatCount(t.conflict);
  el('cntEqual').textContent = formatCount(t.equal);
  el('totalBytes').textContent = t.bytes ? `${formatBytes(t.bytes)} to copy` : '';
  el('viewFilters').hidden = false;
  document.querySelectorAll<HTMLButtonElement>('.vf').forEach((b) => {
    b.classList.toggle('on', view[b.dataset.key as keyof ViewFilter]);
  });
  el<HTMLButtonElement>('btnSync').disabled = !s.canSync;

  renderNotices(s);
  grid.reset(s.rowCount, keepScroll);
  if (s.rowCount === 0) {
    const anyBlocked = s.pairs.some((p) => p.blocked);
    if (anyBlocked) showEmpty('Synchronization is not possible', 'See the message above the list.');
    else if (t.create + t.update + t.del + t.conflict === 0)
      showEmpty('Everything is up to date', 'Left and right are already in sync. There is nothing to copy or delete.');
    else showEmpty('No rows to show', 'All rows are hidden by the view buttons at the bottom.');
  } else {
    el('gridEmpty').hidden = true;
  }
  // Tell the user that identical items exist but are hidden, with a one-click way to show them.
  const hint = el('hiddenHint');
  if (!view.showEqual && t.equal > 0) {
    hint.innerHTML = `${formatCount(t.equal)} identical files and folders are hidden, because they are already the same on both sides. <button class="btn-link" id="btnShowEqual">Show them</button>`;
    hint.hidden = false;
    el('btnShowEqual').onclick = () => document.querySelector<HTMLButtonElement>('.vf[data-key="showEqual"]')?.click();
  } else {
    hint.hidden = true;
  }
  alignGridHeader();
}

/** Applies a direction to the selected rows (right-click menu, Edit menu, Alt+arrow keys). */
async function applyDirection(direction: Direction): Promise<void> {
  if (!summary || syncing || comparing || !grid.hasSelection()) return;
  const sel = grid.selection();
  summary = await api.setDirection(sel.ids, direction, sel.all);
  applySummary(true);
}

function renderNotices(s: Summary): void {
  const box = el('notices');
  let html = '';
  for (const w of s.warnings) html += `<div class="notice notice-warn">${esc(w)}</div>`;
  s.pairs.forEach((p, i) => {
    const label = s.pairs.length > 1 ? `<b>Folder pair ${i + 1}:</b> ` : '';
    if (p.blocked) html += `<div class="notice notice-block">${label}${esc(p.blocked)}</div>`;
    else if (!p.rightExists)
      html += `<div class="notice notice-info">${label}The right folder does not exist yet. It will be created.</div>`;
    else if (!p.leftExists)
      html += `<div class="notice notice-info">${label}The left folder does not exist yet. It will be created.</div>`;
    for (const w of p.warnings) html += `<div class="notice notice-warn">${label}${esc(w)}</div>`;
    if (p.errorCount > 0) {
      const list = p.errors
        .slice(0, 5)
        .map((e) => `<li>${esc(e.path)}: ${esc(e.message)}</li>`)
        .join('');
      const more = p.errorCount > 5 ? `<li>…and ${p.errorCount - 5} more</li>` : '';
      html += `<div class="notice notice-warn">${label}Could not read:<ul>${list}${more}</ul></div>`;
    }
  });
  box.innerHTML = html;
  box.hidden = html === '';
}

document.querySelectorAll<HTMLButtonElement>('.vf').forEach((b) =>
  b.addEventListener('click', async () => {
    const key = b.dataset.key as keyof ViewFilter;
    view = { ...view, [key]: !view[key] };
    b.classList.toggle('on', view[key]);
    if (!summary) return;
    summary.rowCount = await api.setView(view);
    applySummary(false);
  }),
);

// Right-click menu
const ctxMenu = el('ctxMenu');
let ctxRows: GridRow[] = [];
grid.onContextMenu = (_ids, rows, x, y) => {
  ctxRows = rows;
  ctxMenu.hidden = false;
  const w = ctxMenu.offsetWidth;
  const h = ctxMenu.offsetHeight;
  ctxMenu.style.left = `${Math.min(x, window.innerWidth - w - 4)}px`;
  ctxMenu.style.top = `${Math.min(y, window.innerHeight - h - 4)}px`;
  const one = rows.length === 1 ? rows[0] : null;
  ctxMenu.querySelector<HTMLButtonElement>('[data-open="left"]')!.disabled = !one?.left;
  ctxMenu.querySelector<HTMLButtonElement>('[data-open="right"]')!.disabled = !one?.right;
};
document.addEventListener('mousedown', (e) => {
  if (!ctxMenu.hidden && !ctxMenu.contains(e.target as Node)) ctxMenu.hidden = true;
});
ctxMenu.querySelectorAll<HTMLButtonElement>('[data-dir]').forEach((b) =>
  b.addEventListener('click', () => {
    ctxMenu.hidden = true;
    void applyDirection(b.dataset.dir as Direction);
  }),
);
ctxMenu.querySelectorAll<HTMLButtonElement>('[data-open]').forEach((b) =>
  b.addEventListener('click', async () => {
    ctxMenu.hidden = true;
    const row = ctxRows[0];
    if (!row || !summary) return;
    const pair = summary.pairs[row.pair];
    const base = b.dataset.open === 'left' ? pair.left : pair.right;
    try {
      await api.reveal(`${base.replace(/[\\/]+$/, '')}\\${row.path}`);
    } catch (e) {
      await message('Cannot open', String(e));
    }
  }),
);

// ---------------------------------------------------------------------------
// Synchronize
// ---------------------------------------------------------------------------

const variantText: Record<SyncVariant, string> = {
  mirror: '<b>Mirror ⟶</b> The right side becomes an exact copy of the left side.',
  update: '<b>Update ⟶</b> New and changed files are copied to the right. Nothing is deleted.',
  twoWay: '<b>Two-way ⟷</b> Changes on either side are copied to the other side.',
  custom: '<b>Custom</b> Your own direction for each kind of difference.',
};

function openConfirm(): void {
  if (!summary || !summary.canSync || syncing || comparing) return;
  const s = summary;
  el('confirmVariant').innerHTML = variantText[prefs.syncVariant];
  const sum = (f: (p: (typeof s.pairs)[0]) => number) => s.pairs.filter((p) => !p.blocked).reduce((a, p) => a + f(p), 0);
  const rows: [string, number, number][] = [
    ['New files to copy', sum((p) => p.stats.left.createFiles), sum((p) => p.stats.right.createFiles)],
    ['New folders to create', sum((p) => p.stats.left.createFolders), sum((p) => p.stats.right.createFolders)],
    ['Files to overwrite', sum((p) => p.stats.left.updateFiles), sum((p) => p.stats.right.updateFiles)],
    ['Files to delete', sum((p) => p.stats.left.deleteFiles), sum((p) => p.stats.right.deleteFiles)],
    ['Folders to delete', sum((p) => p.stats.left.deleteFolders), sum((p) => p.stats.right.deleteFolders)],
  ];
  const bytesL = sum((p) => p.stats.left.bytesToWrite);
  const bytesR = sum((p) => p.stats.right.bytesToWrite);
  el('confirmTable').innerHTML =
    '<tr><th></th><th>Left</th><th>Right</th></tr>' +
    rows
      .map(
        ([label, l, r]) =>
          `<tr class="${label.includes('delete') && l + r > 0 ? 'row-delete' : ''}"><td>${label}</td><td>${formatCount(l)}</td><td>${formatCount(r)}</td></tr>`,
      )
      .join('') +
    `<tr><td>Data to write</td><td>${formatBytes(bytesL)}</td><td>${formatBytes(bytesR)}</td></tr>`;

  let warn = '';
  const conflicts = sum((p) => p.stats.conflicts);
  if (conflicts) warn += `<div class="notice notice-warn">${conflicts} conflict(s) will be skipped.</div>`;
  s.pairs.forEach((p, i) => {
    if (p.blocked)
      warn += `<div class="notice notice-block">Folder pair ${i + 1} will be skipped: ${esc(p.blocked.split('\n')[0])}</div>`;
    for (const w of p.warnings) if (w.includes('deleted')) warn += `<div class="notice notice-warn">${esc(w)}</div>`;
  });
  el('confirmWarnings').innerHTML = warn;

  el<HTMLSelectElement>('selDeletion').value = prefs.deletion;
  el<HTMLInputElement>('inpVersions').value = prefs.versionsFolder;
  el<HTMLInputElement>('chkVerify').checked = prefs.verify;
  updateDeletionRow();
  showModal('confirmModal');
  el('btnConfirmStart').focus();
}

function updateDeletionRow(): void {
  const v = el<HTMLSelectElement>('selDeletion').value;
  el('rowVersions').hidden = v !== 'versioning';
  el('hintDeletion').hidden = v !== 'recycleBin';
}

el('selDeletion').addEventListener('change', updateDeletionRow);
el('btnVersionsBrowse').addEventListener('click', async () => {
  const picked = await api.pickFolder('Select the versions folder', el<HTMLInputElement>('inpVersions').value || undefined);
  if (picked) el<HTMLInputElement>('inpVersions').value = picked;
});
el('btnConfirmCancel').addEventListener('click', () => hideModal('confirmModal'));
el('btnConfirmStart').addEventListener('click', async () => {
  prefs.deletion = el<HTMLSelectElement>('selDeletion').value as Prefs['deletion'];
  prefs.versionsFolder = el<HTMLInputElement>('inpVersions').value.trim();
  prefs.verify = el<HTMLInputElement>('chkVerify').checked;
  if (prefs.deletion === 'versioning' && !prefs.versionsFolder) {
    await message('Versions folder missing', 'Please choose a versions folder, or pick another option.');
    return;
  }
  savePrefs();
  hideModal('confirmModal');
  void runSync();
});

// Progress
let speedSamples: [number, number][] = [];
let paused = false;

function resetProgress(): void {
  speedSamples = [];
  paused = false;
  el('btnPause').textContent = 'Pause';
  el('progTitle').textContent = 'Synchronizing…';
  el('progPair').textContent = '';
  el('progFill').style.width = '0%';
  el('progItems').textContent = '';
  el('progBytes').textContent = '';
  el('progSpeed').textContent = '';
  el('progEta').textContent = '';
  el('progCurrent').textContent = 'Preparing…';
}

function onSyncEvent(e: SyncEvent): void {
  if (e.type === 'progress') {
    const p = e.progress;
    if (e.pairCount > 1) el('progPair').textContent = `Folder pair ${e.pairIndex + 1} of ${e.pairCount}`;
    const fItems = p.itemsTotal ? p.itemsDone / p.itemsTotal : 1;
    const fBytes = p.bytesTotal ? p.bytesDone / p.bytesTotal : fItems;
    const frac = p.bytesTotal ? 0.85 * fBytes + 0.15 * fItems : fItems;
    el('progFill').style.width = `${(Math.min(1, frac) * 100).toFixed(1)}%`;
    el('progItems').textContent = `${formatCount(p.itemsDone)} of ${formatCount(p.itemsTotal)} items`;
    el('progBytes').textContent = p.bytesTotal ? `${formatBytes(p.bytesDone)} of ${formatBytes(p.bytesTotal)}` : '';
    const now = performance.now();
    speedSamples.push([now, p.bytesDone]);
    while (speedSamples.length > 2 && now - speedSamples[0][0] > 5000) speedSamples.shift();
    const [t0, b0] = speedSamples[0];
    const speed = now > t0 ? ((p.bytesDone - b0) * 1000) / (now - t0) : 0;
    if (!paused) {
      el('progSpeed').textContent = speed > 0 ? `${formatBytes(speed)}/s` : '';
      el('progEta').textContent =
        speed > 0 && p.bytesTotal ? `about ${formatDuration((p.bytesTotal - p.bytesDone) / speed)} left` : '';
    }
    const verb = actionVerb[p.currentAction];
    el('progCurrent').textContent = p.currentPath ? `${verb} ${p.currentPath}` : '';
  } else if (e.type === 'error') {
    el('errPath').textContent = e.failure.relPath;
    el('errMsg').textContent = e.failure.message;
    showModal('errorModal');
    el('errorModal').querySelector<HTMLButtonElement>('[data-decision="retry"]')!.focus();
  }
}

el('errorModal')
  .querySelectorAll<HTMLButtonElement>('[data-decision]')
  .forEach((b) =>
    b.addEventListener('click', () => {
      hideModal('errorModal');
      void api.answerError(b.dataset.decision as ErrorDecision);
      if (b.dataset.decision === 'cancel') el('progTitle').textContent = 'Cancelling…';
    }),
  );

el('btnPause').addEventListener('click', async () => {
  paused = !paused;
  await api.setPaused(paused);
  el('btnPause').textContent = paused ? 'Resume' : 'Pause';
  el('progTitle').textContent = paused ? 'Paused' : 'Synchronizing…';
  if (paused) {
    el('progSpeed').textContent = '';
    el('progEta').textContent = '';
  }
  speedSamples = [];
});

el('btnCancelSync').addEventListener('click', async () => {
  el('progTitle').textContent = 'Cancelling…';
  await api.cancel();
});

async function runSync(): Promise<void> {
  syncing = true;
  el<HTMLButtonElement>('btnSync').disabled = true;
  el<HTMLButtonElement>('btnCompare').disabled = true;
  resetProgress();
  showModal('progressModal');
  setStatus('Synchronizing…');
  let outcome: SyncOutcome | null = null;
  let failure: string | null = null;
  try {
    outcome = await api.startSync(execOptions(), jobName(), onSyncEvent);
  } catch (e) {
    failure = String(e);
  } finally {
    syncing = false;
    hideModal('progressModal');
    hideModal('errorModal');
    el<HTMLButtonElement>('btnCompare').disabled = false;
  }
  showResult(outcome, failure);
}

function showResult(o: SyncOutcome | null, failure: string | null): void {
  const title = el('resTitle');
  const summaryBox = el('resSummary');
  const list = el('resList');
  list.innerHTML = '';
  if (!o) {
    title.textContent = 'Synchronization did not start';
    summaryBox.innerHTML = `<div class="notice notice-block">${esc(failure ?? 'Unknown error')}</div>`;
    setStatus('Synchronization failed');
  } else {
    const failures = o.reports.flatMap((r) => r.failures);
    const d = { copied: 0, updated: 0, deleted: 0, bytes: 0, ms: 0 };
    for (const r of o.reports) {
      d.copied += r.done.createLeft + r.done.createRight;
      d.updated += r.done.updateLeft + r.done.updateRight;
      d.deleted += r.done.deleteLeft + r.done.deleteRight;
      d.bytes += r.bytesCopied;
      d.ms += r.durationMs;
    }
    title.textContent = o.failure
      ? 'Synchronization stopped'
      : o.cancelled
        ? 'Synchronization cancelled'
        : failures.length
        ? `Finished with ${failures.length} problem(s)`
        : 'Synchronization complete';
    summaryBox.innerHTML = `
      <div class="res-big ${o.failure || o.cancelled || failures.length ? 'res-warn' : 'res-ok'}">${o.failure ? '⚠' : o.cancelled ? '⏹' : failures.length ? '⚠' : '✓'}</div>
      <table class="confirm-table">
        <tr><td>New items copied</td><td>${formatCount(d.copied)}</td></tr>
        <tr><td>Items overwritten</td><td>${formatCount(d.updated)}</td></tr>
        <tr><td>Items deleted</td><td>${formatCount(d.deleted)}</td></tr>
        <tr><td>Data copied</td><td>${formatBytes(d.bytes)}</td></tr>
        <tr><td>Time</td><td>${formatDuration(d.ms / 1000)}</td></tr>
      </table>`;
    let html = '';
    if (o.failure) html += `<div class="notice notice-block">${esc(o.failure)}</div>`;
    for (const s of o.skipped) html += `<div class="notice notice-block">Skipped: ${esc(s)}</div>`;
    for (const r of o.reports) for (const n of r.notes) html += `<div class="notice notice-info">${esc(n)}</div>`;
    if (failures.length) {
      html += '<h3>Problems</h3><ul class="fail-list">';
      html += failures
        .slice(0, 300)
        .map((f) => `<li><b>${esc(f.relPath)}</b><br>${esc(f.message)}</li>`)
        .join('');
      if (failures.length > 300) html += `<li>…and ${failures.length - 300} more</li>`;
      html += '</ul>';
    }
    if (o.logFile) html += `<p class="res-log hint">Saved in the log. <button class="btn-link" id="btnResLog">Show log</button></p>`;
    list.innerHTML = html;
    const logFile = o.logFile;
    if (logFile) el('btnResLog').onclick = () => void openLogs(logFile);
    setStatus(title.textContent);
  }
  showModal('resultModal');
  el('btnResultClose').focus();
}

el('btnResultClose').addEventListener('click', () => {
  hideModal('resultModal');
  // Show what is left (normally nothing) by comparing again.
  void runCompare();
});

el('btnCompare').addEventListener('click', () => void runCompare());
el('btnSync').addEventListener('click', openConfirm);

// ---------------------------------------------------------------------------
// About, help, filter, keyboard
// ---------------------------------------------------------------------------

async function showAbout(): Promise<void> {
  try {
    const info = await api.appInfo();
    el('aboutVersion').textContent = `Version ${info.version} · 64-bit`;
  } catch {
    // keep the default text
  }
  showModal('aboutModal');
  el('btnAboutClose').focus();
}

el('btnAboutClose').addEventListener('click', () => hideModal('aboutModal'));
document.querySelectorAll<HTMLAnchorElement>('[data-url]').forEach((a) =>
  a.addEventListener('click', (e) => {
    e.preventDefault();
    if (a.dataset.url) void api.openLink(a.dataset.url);
  }),
);

function showGuide(): void {
  void message(
    'How to use Relax',
    [
      '1. Choose the left (source) and right (target) folders. Drag a folder from Explorer onto a box, or click Browse.',
      '2. Choose a sync mode:',
      '    Mirror: the right side becomes an exact copy of the left.',
      '    Update: only new and changed files are copied; nothing is deleted (incremental backup).',
      '    Two-way: changes on either side are copied to the other.',
      '3. Click Compare (F5) and check the list. Right-click rows to change what happens to them.',
      '4. Click Synchronize (F9).',
      '',
      'The contents of the left folder are copied into the right folder; no extra folder is created inside it.',
      '',
      'Backing up to two drives: click "+ Add folder pair" and use the same left folder with a second right folder.',
      '',
      'Saved jobs are listed on the left. Double-click a job to open and compare it; right-click to rename or delete it.',
      'Deleted and overwritten files go to the Recycle Bin by default (to a Relax-Versions folder on USB sticks).',
      '',
      'Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.',
    ].join('\n'),
    true,
  );
}

function showShortcuts(): void {
  void message(
    'Keyboard shortcuts',
    [
      'Ctrl+N    New job',
      'Ctrl+O    Open job',
      'Ctrl+S    Save job          Ctrl+Shift+S   Save job as',
      'F5        Compare',
      'F9        Synchronize',
      'Ctrl+F    Filter            Ctrl+,         Settings',
      'Ctrl+L    Sync logs         Ctrl+J         Show / hide saved jobs',
      'Ctrl+A    Select all rows',
      'Alt+Right Make right like left',
      'Alt+Left  Make left like right',
      'Alt+Down  Do nothing',
      'Alt+F, Alt+E, Alt+T, Alt+H   Open the menus',
      'F1        User guide',
    ].join('\n'),
  );
}

function swapAllPairs(): void {
  for (const p of prefs.pairs) [p.left, p.right] = [p.right, p.left];
  savePrefs();
  renderPairs();
  markStale();
}

async function clearPairs(): Promise<void> {
  const r = await ask({ title: 'Clear all folders', text: 'Remove all folder pairs from this job?', okLabel: 'Clear' });
  if (r.button !== 'ok') return;
  prefs.pairs = [{ left: '', right: '' }];
  savePrefs();
  renderPairs();
  markStale();
}

const idle = () => !syncing && !comparing;
const commands: Commands = {
  jobNew: cmdNewJob,
  jobOpen: cmdOpenJob,
  jobSave: cmdSaveJob,
  jobSaveAs: cmdSaveJobAs,
  jobsFolder: () =>
    void invoke<string>('default_jobs_dir')
      .then((d) => api.reveal(d))
      .catch((e) => message('Jobs folder', String(e))),
  exit: () => void getCurrentWindow().close(),
  addPair: () => el('btnAddPair').click(),
  swapAll: swapAllPairs,
  clearPairs: () => void clearPairs(),
  selectAll: () => grid.selectAll(),
  dirRight: () => void applyDirection('toRight'),
  dirLeft: () => void applyDirection('toLeft'),
  dirNone: () => void applyDirection('none'),
  compare: () => void runCompare(),
  sync: openConfirm,
  filter: () => idle() && openFilter(settingsChanged),
  settings: () => idle() && openSettings(settingsChanged),
  schedule: cmdSchedule,
  logs: () => void openLogs(),
  togglePane,
  guide: showGuide,
  shortcuts: showShortcuts,
  website: () => void api.openLink('https://vsa.edu.in/'),
  email: () => void api.openLink('mailto:softinquiry@vsa.edu.in'),
  github: () => void api.openLink('https://github.com/dsvakola/relax'),
  privacy: () => void api.openLink('https://github.com/dsvakola/relax/blob/main/PRIVACY.md'),
  about: () => void showAbout(),
};

initMenus(commands, (cmd) => {
  switch (cmd) {
    case 'sync':
      return idle() && !!summary?.canSync;
    case 'selectAll':
    case 'dirRight':
    case 'dirLeft':
    case 'dirNone':
      return idle() && !!summary && (cmd === 'selectAll' || grid.hasSelection());
    case 'compare':
      return !syncing;
    case 'jobNew':
    case 'jobOpen':
    case 'jobSave':
    case 'jobSaveAs':
    case 'filter':
    case 'settings':
    case 'schedule':
    case 'addPair':
    case 'swapAll':
    case 'clearPairs':
      return idle();
    default:
      return true;
  }
});

initJobsPane({
  knownJobs,
  currentJob: () => prefs.jobPath,
  open: (path, thenCompare) => void openJobFromPane(path, thenCompare),
  forget: (path) => {
    forgetJob(path);
    void refreshScheduleDot();
  },
  renamed: (oldPath, newPath) => {
    jobRenamed(oldPath, newPath);
    void updateTitle();
  },
  message,
});

const anyDialogOpen = () => [...document.querySelectorAll<HTMLElement>('.modal-backdrop')].some((m) => !m.hidden);

document.addEventListener('keydown', (e) => {
  const inText = (e.target as HTMLElement).closest('input, textarea, select') !== null;
  const key = e.key.toLowerCase();
  const run = (cmd: string) => {
    e.preventDefault();
    commands[cmd]?.();
  };
  if (e.key === 'Escape') {
    ctxMenu.hidden = true;
    el('jobMenu').hidden = true;
    for (const id of ['aboutModal', 'confirmModal', 'settingsModal', 'filterModal', 'scheduleModal', 'logsModal'])
      hideModal(id);
    return;
  }
  if (anyDialogOpen()) return;
  if (e.key === 'F5') run('compare');
  else if (e.key === 'F9') run('sync');
  else if (e.key === 'F1') run('guide');
  else if (e.ctrlKey && key === 's') run(e.shiftKey ? 'jobSaveAs' : 'jobSave');
  else if (e.ctrlKey && key === 'o') run('jobOpen');
  else if (e.ctrlKey && key === 'n') run('jobNew');
  else if (e.ctrlKey && key === 'f') run('filter');
  else if (e.ctrlKey && key === ',') run('settings');
  else if (e.ctrlKey && key === 'l') run('logs');
  else if (e.ctrlKey && key === 'j') run('togglePane');
  else if (e.ctrlKey && key === 'a' && !inText) run('selectAll');
  else if (e.altKey && e.key === 'ArrowRight' && !inText) run('dirRight');
  else if (e.altKey && e.key === 'ArrowLeft' && !inText) run('dirLeft');
  else if (e.altKey && e.key === 'ArrowDown' && !inText) run('dirNone');
});

// Block the browser's own context menu; it makes no sense in a desktop app.
document.addEventListener('contextmenu', (e) => {
  if (!(e.target as HTMLElement).closest('input, textarea')) e.preventDefault();
});

// Closing the window with unsaved changes: offer to save first.
void getCurrentWindow().onCloseRequested(async (event) => {
  if (!isDirty() || syncing) {
    if (syncing) {
      event.preventDefault();
      await message('Synchronization running', 'Please wait for the synchronization to finish, or cancel it first.');
    }
    return;
  }
  event.preventDefault();
  if (await confirmLeaveJob()) await getCurrentWindow().destroy();
});

showPrefs();
void updateTitle();
void refreshScheduleDot();
void refreshJobs();
initLicense();
// Started by double-clicking a .relax file: load that job.
void api.startupJob().then(async (path) => {
  if (!path) return;
  try {
    await loadJobFromPath(path);
    showPrefs();
    void refreshScheduleDot();
    void refreshJobs();
    setStatus(`Opened: ${path}`);
  } catch (e) {
    await message('Job file', String(e instanceof Error ? e.message : e));
  }
});
alignGridHeader();
// Show the window only after the first paint, so there is no white flash.
requestAnimationFrame(() => void getCurrentWindow().show());
