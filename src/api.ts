// Relax - Your data is safe!
// Typed bridge to the Rust backend. Every backend call goes through this file.
// Types mirror the Rust structs in src-tauri/src (serde camelCase).
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

import { Channel, invoke } from '@tauri-apps/api/core';
import { open } from '@tauri-apps/plugin-dialog';
import { openUrl } from '@tauri-apps/plugin-opener';

export interface AppInfo {
  name: string;
  version: string;
  developedBy: string;
  organization: string;
  website: string;
  email: string;
}

/** `free` = not a Microsoft Store install; `unknown` = the Store cannot be asked and nothing is saved (Relax stays usable). */
export interface LicenseInfo {
  state: 'free' | 'trial' | 'licensed' | 'expired' | 'unknown';
  daysLeft: number | null;
  price: string;
}

export type CompareVariant = 'timeAndSize' | 'content' | 'size';
export type SyncVariant = 'twoWay' | 'mirror' | 'update' | 'custom';
export type Direction = 'toRight' | 'toLeft' | 'none';
export type Category = 'equal' | 'leftOnly' | 'rightOnly' | 'leftNewer' | 'rightNewer' | 'different' | 'conflict';
export type Action =
  | 'none'
  | 'createRight'
  | 'createLeft'
  | 'updateRight'
  | 'updateLeft'
  | 'deleteRight'
  | 'deleteLeft'
  | 'conflict';
export type ItemKind = 'file' | 'folder';
export type ErrorDecision = 'retry' | 'ignore' | 'ignoreAll' | 'cancel';

export interface FolderPair {
  left: string;
  right: string;
}

export interface DirectionTable {
  leftOnly: Direction;
  rightOnly: Direction;
  leftNewer: Direction;
  rightNewer: Direction;
  different: Direction;
  conflict: Direction;
}

export type DeletionPolicy = { kind: 'recycleBin' } | { kind: 'versioning'; folder: string } | { kind: 'permanent' };

export interface JobConfig {
  pairs: FolderPair[];
  compare: { variant: CompareVariant; timeToleranceSecs: number; ignoreHourShift: boolean };
  filter: { include: string[]; exclude: string[] };
  sync: { variant: SyncVariant; custom: DirectionTable; allowSyncFromEmptyFolder: boolean; timeToleranceSecs: number };
  exec: ExecOptions;
}

export interface ExecOptions {
  deletion: DeletionPolicy;
  verifyCopies: boolean;
}

export interface ViewFilter {
  showEqual: boolean;
  showSkipped: boolean;
  showCreate: boolean;
  showUpdate: boolean;
  showDelete: boolean;
  showConflict: boolean;
}

export interface SideStats {
  createFiles: number;
  createFolders: number;
  updateFiles: number;
  deleteFiles: number;
  deleteFolders: number;
  bytesToWrite: number;
}

export interface PlanStats {
  left: SideStats;
  right: SideStats;
  conflicts: number;
  unchanged: number;
}

export interface CompareStats {
  equal: number;
  leftFiles: number;
  leftFolders: number;
  leftBytes: number;
  rightFiles: number;
  rightFolders: number;
  rightBytes: number;
}

export interface ItemError {
  side: 'left' | 'right';
  path: string;
  message: string;
}

export interface PairSummary {
  left: string;
  right: string;
  leftExists: boolean;
  rightExists: boolean;
  variant: SyncVariant;
  stats: PlanStats;
  compareStats: CompareStats;
  warnings: string[];
  blocked: string | null;
  errors: ItemError[];
  errorCount: number;
}

export interface Summary {
  pairs: PairSummary[];
  warnings: string[];
  rowCount: number;
  canSync: boolean;
  elapsedMs: number;
}

export interface Cell {
  name: string;
  size: number;
  modifiedMs: number;
}

export interface GridRow {
  id: number;
  header: boolean;
  pair: number;
  path: string;
  depth: number;
  kind: ItemKind | null;
  left: Cell | null;
  right: Cell | null;
  category: Category | null;
  action: Action | null;
  note: string | null;
}

export type CompareProgress =
  | { phase: 'scanning'; pairIndex: number; itemsFound: number }
  | { phase: 'comparingContent'; pairIndex: number; bytesDone: number; bytesTotal: number };

export interface ExecProgress {
  itemsDone: number;
  itemsTotal: number;
  bytesDone: number;
  bytesTotal: number;
  currentPath: string;
  currentAction: Action;
}

export interface ItemFailure {
  relPath: string;
  action: Action;
  message: string;
}

export interface ExecReport {
  left: string;
  right: string;
  done: {
    createLeft: number;
    createRight: number;
    updateLeft: number;
    updateRight: number;
    deleteLeft: number;
    deleteRight: number;
  };
  bytesCopied: number;
  failures: ItemFailure[];
  notes: string[];
  cancelled: boolean;
  durationMs: number;
}

export type SyncEvent =
  | { type: 'progress'; pairIndex: number; pairCount: number; progress: ExecProgress }
  | { type: 'error'; pairIndex: number; failure: ItemFailure }
  | { type: 'pairDone'; pairIndex: number; report: ExecReport };

export interface SyncOutcome {
  reports: ExecReport[];
  skipped: string[];
  cancelled: boolean;
  failure: string | null;
  logFile: string | null;
}

export interface Schedule {
  kind: 'daily' | 'weekly' | 'logon';
  time: string;
  day: string;
}

export interface ScheduleInfo {
  exists: boolean;
  nextRun: string;
  taskName: string;
}

export const api = {
  appInfo: (): Promise<AppInfo> => invoke<AppInfo>('app_info'),
  getLicense: (): Promise<LicenseInfo> => invoke<LicenseInfo>('get_license'),
  buyApp: (): Promise<LicenseInfo> => invoke<LicenseInfo>('buy_app'),

  compare: (job: JobConfig, onProgress: (p: CompareProgress) => void): Promise<Summary> => {
    const ch = new Channel<CompareProgress>();
    ch.onmessage = onProgress;
    return invoke<Summary>('compare_folders', { job, onProgress: ch });
  },

  setView: (view: ViewFilter): Promise<number> => invoke<number>('set_view', { view }),

  getRows: (offset: number, limit: number): Promise<GridRow[]> => invoke<GridRow[]>('get_rows', { offset, limit }),

  setDirection: (ids: number[], direction: Direction, allVisible = false): Promise<Summary> =>
    invoke<Summary>('set_direction', { ids, direction, allVisible }),

  startSync: (exec: ExecOptions, jobName: string, onEvent: (e: SyncEvent) => void): Promise<SyncOutcome> => {
    const ch = new Channel<SyncEvent>();
    ch.onmessage = onEvent;
    return invoke<SyncOutcome>('start_sync', { exec, jobName: jobName || null, onEvent: ch });
  },

  listLogs: (): Promise<string[]> => invoke<string[]>('list_logs'),
  readLog: (name: string): Promise<string> => invoke<string>('read_log', { name }),
  openLogsFolder: (): Promise<void> => invoke('open_logs_folder'),
  getSchedule: (jobPath: string): Promise<ScheduleInfo> => invoke<ScheduleInfo>('get_schedule', { jobPath }),
  createSchedule: (jobPath: string, schedule: Schedule): Promise<ScheduleInfo> =>
    invoke<ScheduleInfo>('create_schedule', { jobPath, schedule }),
  deleteSchedule: (jobPath: string): Promise<void> => invoke('delete_schedule', { jobPath }),
  startupJob: (): Promise<string | null> => invoke<string | null>('startup_job'),

  answerError: (decision: ErrorDecision): Promise<void> => invoke('answer_error', { decision }),
  setPaused: (paused: boolean): Promise<void> => invoke('set_paused', { paused }),
  cancel: (): Promise<void> => invoke('cancel_operation'),
  reveal: (path: string): Promise<void> => invoke('reveal_path', { path }),

  /** Opens a folder picker. Returns null if the user cancels. */
  pickFolder: async (title: string, defaultPath?: string): Promise<string | null> => {
    const picked = await open({ directory: true, multiple: false, title, defaultPath });
    return typeof picked === 'string' ? picked : null;
  },

  openLink: (url: string): Promise<void> => openUrl(url),
};
