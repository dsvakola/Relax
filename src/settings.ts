// Relax - settings: remembered preferences, job files (.relax), and the Filter and Settings windows.
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

import { invoke } from '@tauri-apps/api/core';
import { open, save } from '@tauri-apps/plugin-dialog';
import { getCurrentWindow } from '@tauri-apps/api/window';
import {
  api,
  type CompareVariant,
  type DeletionPolicy,
  type Direction,
  type DirectionTable,
  type ExecOptions,
  type FolderPair,
  type JobConfig,
  type SyncVariant,
} from './api';

export interface Prefs {
  pairs: FolderPair[];
  compareVariant: CompareVariant;
  timeTolerance: number;
  ignoreHourShift: boolean;
  include: string[];
  exclude: string[];
  syncVariant: SyncVariant;
  custom: DirectionTable;
  allowEmpty: boolean;
  deletion: 'recycleBin' | 'versioning' | 'permanent';
  versionsFolder: string;
  verify: boolean;
  /** Job file the settings came from (empty if none). Not stored inside job files. */
  jobPath: string;
}

export const DEFAULT_EXCLUDE = [
  '\\System Volume Information\\',
  '\\$Recycle.Bin\\',
  '\\RECYCLER\\',
  '\\RECYCLED\\',
  'Thumbs.db',
  'desktop.ini',
];

const PRESETS: Record<'mirror' | 'update', DirectionTable> = {
  mirror: {
    leftOnly: 'toRight',
    rightOnly: 'toRight',
    leftNewer: 'toRight',
    rightNewer: 'toRight',
    different: 'toRight',
    conflict: 'toRight',
  },
  update: {
    leftOnly: 'toRight',
    rightOnly: 'none',
    leftNewer: 'toRight',
    rightNewer: 'none',
    different: 'toRight',
    conflict: 'none',
  },
};

const defaults = (): Prefs => ({
  pairs: [{ left: '', right: '' }],
  compareVariant: 'timeAndSize',
  timeTolerance: 2,
  ignoreHourShift: false,
  include: ['*'],
  exclude: [...DEFAULT_EXCLUDE],
  syncVariant: 'mirror',
  custom: { ...PRESETS.mirror },
  allowEmpty: false,
  deletion: 'recycleBin',
  versionsFolder: '',
  verify: false,
  jobPath: '',
});

const PREFS_KEY = 'relax.prefs.v1';

/** Merges stored or loaded values over the defaults, ignoring anything malformed. */
function sanitize(raw: unknown): Prefs {
  const p = defaults();
  if (!raw || typeof raw !== 'object') return p;
  const r = raw as Record<string, unknown>;
  const str = (v: unknown, d: string) => (typeof v === 'string' ? v : d);
  const bool = (v: unknown, d: boolean) => (typeof v === 'boolean' ? v : d);
  const list = (v: unknown, d: string[]) =>
    Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : d;
  if (Array.isArray(r.pairs)) {
    const pairs = r.pairs
      .filter((x): x is FolderPair => !!x && typeof x === 'object')
      .map((x) => ({ left: str(x.left, ''), right: str(x.right, '') }));
    if (pairs.length) p.pairs = pairs;
  }
  if (['timeAndSize', 'content', 'size'].includes(r.compareVariant as string))
    p.compareVariant = r.compareVariant as CompareVariant;
  if (typeof r.timeTolerance === 'number' && r.timeTolerance >= 0 && r.timeTolerance <= 3600)
    p.timeTolerance = Math.round(r.timeTolerance);
  p.ignoreHourShift = bool(r.ignoreHourShift, p.ignoreHourShift);
  p.include = list(r.include, p.include);
  p.exclude = list(r.exclude, p.exclude);
  if (['twoWay', 'mirror', 'update', 'custom'].includes(r.syncVariant as string))
    p.syncVariant = r.syncVariant as SyncVariant;
  if (r.custom && typeof r.custom === 'object') {
    const c = r.custom as Record<string, unknown>;
    for (const k of Object.keys(p.custom) as (keyof DirectionTable)[]) {
      if (['toRight', 'toLeft', 'none'].includes(c[k] as string)) p.custom[k] = c[k] as Direction;
    }
  }
  p.allowEmpty = bool(r.allowEmpty, p.allowEmpty);
  if (['recycleBin', 'versioning', 'permanent'].includes(r.deletion as string))
    p.deletion = r.deletion as Prefs['deletion'];
  p.versionsFolder = str(r.versionsFolder, p.versionsFolder);
  p.verify = bool(r.verify, p.verify);
  p.jobPath = str(r.jobPath, p.jobPath);
  return p;
}

function loadPrefs(): Prefs {
  try {
    const raw = localStorage.getItem(PREFS_KEY);
    if (raw) return sanitize(JSON.parse(raw));
  } catch {
    // unreadable preferences: start with defaults
  }
  return defaults();
}

export const prefs: Prefs = loadPrefs();

export function savePrefs(): void {
  try {
    localStorage.setItem(PREFS_KEY, JSON.stringify(prefs));
  } catch {
    // storage unavailable: preferences are simply not remembered
  }
  // The title shows "*" while the job has unsaved changes.
  if (typeof updateTitle === 'function') void updateTitle();
}

export function directionTable(): DirectionTable {
  if (prefs.syncVariant === 'mirror' || prefs.syncVariant === 'update') return { ...PRESETS[prefs.syncVariant] };
  return { ...prefs.custom };
}

export function execOptions(): ExecOptions {
  let deletion: DeletionPolicy = { kind: 'recycleBin' };
  if (prefs.deletion === 'permanent') deletion = { kind: 'permanent' };
  if (prefs.deletion === 'versioning') deletion = { kind: 'versioning', folder: prefs.versionsFolder };
  return { deletion, verifyCopies: prefs.verify };
}

export function buildJob(pairs: FolderPair[]): JobConfig {
  return {
    pairs,
    compare: {
      variant: prefs.compareVariant,
      timeToleranceSecs: prefs.timeTolerance,
      ignoreHourShift: prefs.ignoreHourShift,
    },
    filter: { include: prefs.include, exclude: prefs.exclude },
    sync: {
      variant: prefs.syncVariant,
      custom: directionTable(),
      allowSyncFromEmptyFolder: prefs.allowEmpty,
      timeToleranceSecs: prefs.timeTolerance,
    },
    exec: execOptions(),
  };
}

export function filterIsDefault(): boolean {
  return (
    prefs.include.length === 1 &&
    prefs.include[0] === '*' &&
    prefs.exclude.length === DEFAULT_EXCLUDE.length &&
    prefs.exclude.every((x, i) => x === DEFAULT_EXCLUDE[i])
  );
}

// ---------------------------------------------------------------------------
// Job files
// ---------------------------------------------------------------------------

const JOB_FORMAT = 1;

export function jobName(): string {
  if (!prefs.jobPath) return '';
  return prefs.jobPath.split(/[\\/]/).pop()!.replace(/\.relax$/i, '');
}

// Job files the user saved or opened, shown in the saved-jobs pane (newest first).
const JOBS_KEY = 'relax.jobs.v1';

function loadLibrary(): string[] {
  try {
    const v = JSON.parse(localStorage.getItem(JOBS_KEY) ?? '[]');
    return Array.isArray(v) ? v.filter((x): x is string => typeof x === 'string') : [];
  } catch {
    return [];
  }
}

let library = loadLibrary();

function saveLibrary(): void {
  try {
    localStorage.setItem(JOBS_KEY, JSON.stringify(library.slice(0, 200)));
  } catch {
    // not remembered
  }
}

export function knownJobs(): string[] {
  return [...library];
}

export function rememberJob(path: string): void {
  library = [path, ...library.filter((p) => p.toLowerCase() !== path.toLowerCase())];
  saveLibrary();
}

export function forgetJob(path: string): void {
  library = library.filter((p) => p.toLowerCase() !== path.toLowerCase());
  saveLibrary();
  if (prefs.jobPath.toLowerCase() === path.toLowerCase()) {
    prefs.jobPath = '';
    savePrefs();
  }
}

export function jobRenamed(oldPath: string, newPath: string): void {
  library = library.map((p) => (p.toLowerCase() === oldPath.toLowerCase() ? newPath : p));
  saveLibrary();
  if (prefs.jobPath.toLowerCase() === oldPath.toLowerCase()) {
    prefs.jobPath = newPath;
    savePrefs();
  }
}

// Unsaved-changes tracking: the job's settings as last saved or loaded.
function snapshot(): string {
  const s: Partial<Prefs> = { ...prefs };
  delete s.jobPath;
  return JSON.stringify(s);
}

let cleanSnapshot = snapshot();

export function isDirty(): boolean {
  return snapshot() !== cleanSnapshot;
}

export async function updateTitle(): Promise<void> {
  const name = jobName() || 'New job';
  const mark = isDirty() ? '*' : '';
  await getCurrentWindow()
    .setTitle(`${mark}${name} - Relax - Your data is safe!`)
    .catch(() => {});
}

async function writeJob(path: string): Promise<void> {
  const settings: Partial<Prefs> = { ...prefs };
  delete settings.jobPath;
  const content = JSON.stringify(
    { app: 'Relax - Your data is safe!', format: JOB_FORMAT, settings },
    null,
    2,
  );
  await invoke('save_job_file', { path, content });
  prefs.jobPath = path;
  cleanSnapshot = snapshot();
  rememberJob(path);
  savePrefs();
}

export async function saveJobAs(): Promise<string | null> {
  let dir = '';
  try {
    dir = await invoke<string>('default_jobs_dir');
  } catch {
    // let the dialog choose
  }
  const fileName = `${jobName() || 'My backup'}.relax`;
  const path = await save({
    title: 'Save Relax job',
    defaultPath: prefs.jobPath || (dir ? `${dir}\\${fileName}` : fileName),
    filters: [{ name: 'Relax job', extensions: ['relax'] }],
  });
  if (!path) return null;
  const withExt = /\.relax$/i.test(path) ? path : `${path}.relax`;
  await writeJob(withExt);
  return withExt;
}

export async function saveJob(): Promise<string | null> {
  if (!prefs.jobPath) return saveJobAs();
  await writeJob(prefs.jobPath);
  return prefs.jobPath;
}

/** Asks for a job file and loads it into `prefs`. Returns the path, or null if cancelled. */
export async function openJob(): Promise<string | null> {
  let dir: string | undefined;
  try {
    dir = await invoke<string>('default_jobs_dir');
  } catch {
    dir = undefined;
  }
  const path = await open({
    title: 'Open Relax job',
    multiple: false,
    directory: false,
    defaultPath: dir,
    filters: [{ name: 'Relax job', extensions: ['relax'] }],
  });
  if (typeof path !== 'string') return null;
  await loadJobFromPath(path);
  return path;
}

/** Loads a job file into `prefs` (also used when Relax is started by double-clicking a .relax file). */
export async function loadJobFromPath(path: string): Promise<void> {
  const text = await invoke<string>('load_job_file', { path });
  let parsed: unknown;
  try {
    parsed = JSON.parse(text);
  } catch {
    throw new Error('This file is not a valid Relax job.');
  }
  const doc = parsed as { app?: string; settings?: unknown };
  if (!doc || typeof doc !== 'object' || !doc.settings) throw new Error('This file is not a valid Relax job.');
  Object.assign(prefs, sanitize(doc.settings), { jobPath: path });
  cleanSnapshot = snapshot();
  rememberJob(path);
  savePrefs();
}

/** Starts a new, empty job (keeps nothing from the current one). */
export function newJob(): void {
  Object.assign(prefs, defaults());
  cleanSnapshot = snapshot();
  savePrefs();
}

// ---------------------------------------------------------------------------
// Filter window
// ---------------------------------------------------------------------------

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing element #${id}`);
  return node as T;
}

const lines = (text: string) =>
  text
    .split(/\r?\n|;/)
    .map((s) => s.trim())
    .filter(Boolean);

export function openFilter(onChanged: () => void): void {
  el<HTMLTextAreaElement>('txtInclude').value = prefs.include.join('\n');
  el<HTMLTextAreaElement>('txtExclude').value = prefs.exclude.join('\n');
  el('filterModal').hidden = false;
  el('txtExclude').focus();
  el('btnFilterDefaults').onclick = () => {
    el<HTMLTextAreaElement>('txtInclude').value = '*';
    el<HTMLTextAreaElement>('txtExclude').value = DEFAULT_EXCLUDE.join('\n');
  };
  el('btnFilterCancel').onclick = () => (el('filterModal').hidden = true);
  el('btnFilterOk').onclick = () => {
    const include = lines(el<HTMLTextAreaElement>('txtInclude').value);
    prefs.include = include.length ? include : ['*'];
    prefs.exclude = lines(el<HTMLTextAreaElement>('txtExclude').value);
    savePrefs();
    el('filterModal').hidden = true;
    onChanged();
  };
}

// ---------------------------------------------------------------------------
// Settings window
// ---------------------------------------------------------------------------

const CATS: [keyof DirectionTable, string][] = [
  ['leftOnly', 'Item exists only on the left'],
  ['rightOnly', 'Item exists only on the right'],
  ['leftNewer', 'Left file is newer'],
  ['rightNewer', 'Right file is newer'],
  ['different', 'Files are different (content or size)'],
  ['conflict', 'Conflict (cannot tell which is newer)'],
];

function radioValue(name: string): string {
  return document.querySelector<HTMLInputElement>(`input[name="${name}"]:checked`)?.value ?? '';
}

function setRadio(name: string, value: string): void {
  const r = document.querySelector<HTMLInputElement>(`input[name="${name}"][value="${value}"]`);
  if (r) r.checked = true;
}

function renderDirections(variant: SyncVariant, table: DirectionTable): void {
  const box = el('dirTable');
  if (variant === 'twoWay') {
    box.innerHTML =
      '<p class="hint">Two-way decides automatically: Relax remembers the state after each sync, so it knows what was added, changed or deleted on each side. Items changed on both sides become conflicts for you to decide.</p>';
    return;
  }
  const editable = variant === 'custom';
  box.innerHTML =
    '<table class="dir-table">' +
    CATS.map(
      ([k, label]) => `<tr><td>${label}</td><td>
        <select data-cat="${k}" ${editable ? '' : 'disabled'}>
          <option value="toRight">→ Make right like left</option>
          <option value="toLeft">← Make left like right</option>
          <option value="none">Do nothing</option>
        </select></td></tr>`,
    ).join('') +
    '</table>' +
    (editable ? '' : '<p class="hint">Choose <b>Custom</b> to change these.</p>');
  box.querySelectorAll<HTMLSelectElement>('select[data-cat]').forEach((s) => {
    s.value = table[s.dataset.cat as keyof DirectionTable];
  });
}

function readDirections(): DirectionTable {
  const t = { ...prefs.custom };
  el('dirTable')
    .querySelectorAll<HTMLSelectElement>('select[data-cat]')
    .forEach((s) => (t[s.dataset.cat as keyof DirectionTable] = s.value as Direction));
  return t;
}

export function openSettings(onChanged: () => void): void {
  setRadio('cmpVariant', prefs.compareVariant);
  el<HTMLInputElement>('inpTolerance').value = String(prefs.timeTolerance);
  el<HTMLInputElement>('chkHourShift').checked = prefs.ignoreHourShift;
  setRadio('syncVariant', prefs.syncVariant);
  const tableFor = (v: SyncVariant) =>
    v === 'mirror' || v === 'update' ? PRESETS[v] : v === 'custom' ? prefs.custom : PRESETS.mirror;
  renderDirections(prefs.syncVariant, tableFor(prefs.syncVariant));
  el<HTMLInputElement>('chkAllowEmpty').checked = prefs.allowEmpty;
  el<HTMLSelectElement>('setDeletion').value = prefs.deletion;
  el<HTMLInputElement>('setVersions').value = prefs.versionsFolder;
  el<HTMLInputElement>('setVerify').checked = prefs.verify;
  const syncDeletionRow = () => (el('setVersionsRow').hidden = el<HTMLSelectElement>('setDeletion').value !== 'versioning');
  syncDeletionRow();

  document.querySelectorAll<HTMLInputElement>('input[name="syncVariant"]').forEach((r) => {
    r.onchange = () => {
      const v = radioValue('syncVariant') as SyncVariant;
      // Switching to Custom starts from what was shown, so it is easy to adjust.
      const shown = v === 'custom' ? { ...readDirectionsOr(tableFor(v)) } : tableFor(v);
      renderDirections(v, shown);
    };
  });
  el('setDeletion').onchange = syncDeletionRow;
  el('btnSetVersions').onclick = async () => {
    const picked = await api.pickFolder('Select the versions folder', el<HTMLInputElement>('setVersions').value || undefined);
    if (picked) el<HTMLInputElement>('setVersions').value = picked;
  };
  el('btnSettingsCancel').onclick = () => (el('settingsModal').hidden = true);
  el('btnSettingsOk').onclick = () => {
    prefs.compareVariant = radioValue('cmpVariant') as CompareVariant;
    const tol = Number(el<HTMLInputElement>('inpTolerance').value);
    prefs.timeTolerance = Number.isFinite(tol) ? Math.min(3600, Math.max(0, Math.round(tol))) : 2;
    prefs.ignoreHourShift = el<HTMLInputElement>('chkHourShift').checked;
    prefs.syncVariant = radioValue('syncVariant') as SyncVariant;
    if (prefs.syncVariant === 'custom') prefs.custom = readDirections();
    prefs.allowEmpty = el<HTMLInputElement>('chkAllowEmpty').checked;
    prefs.deletion = el<HTMLSelectElement>('setDeletion').value as Prefs['deletion'];
    prefs.versionsFolder = el<HTMLInputElement>('setVersions').value.trim();
    prefs.verify = el<HTMLInputElement>('setVerify').checked;
    savePrefs();
    el('settingsModal').hidden = true;
    onChanged();
  };
  el('settingsModal').hidden = false;
}

function readDirectionsOr(fallback: DirectionTable): DirectionTable {
  return el('dirTable').querySelector('select[data-cat]') ? readDirections() : { ...fallback };
}
