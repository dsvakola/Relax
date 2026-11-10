// Relax - number, size, time and label formatting for the screens.

import type { Action } from './api';

export function formatBytes(bytes: number): string {
  if (!bytes) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB'];
  const i = Math.min(units.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)));
  const v = bytes / Math.pow(1024, i);
  return `${i === 0 ? v : v.toFixed(v < 10 ? 2 : v < 100 ? 1 : 0)} ${units[i]}`;
}

const dateFmt = new Intl.DateTimeFormat(undefined, {
  year: 'numeric',
  month: '2-digit',
  day: '2-digit',
  hour: '2-digit',
  minute: '2-digit',
  hourCycle: 'h23',
});

export function formatDate(ms: number): string {
  return dateFmt.format(new Date(ms));
}

export function formatCount(n: number): string {
  return n.toLocaleString();
}

export function formatDuration(seconds: number): string {
  if (!isFinite(seconds) || seconds < 0) return '–';
  const s = Math.round(seconds);
  if (s < 60) return `${s} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m} min ${s % 60} s`;
  return `${Math.floor(m / 60)} h ${m % 60} min`;
}

export const actionLabel: Record<Action, string> = {
  none: 'No change',
  createRight: 'Copy new item to the right',
  createLeft: 'Copy new item to the left',
  updateRight: 'Overwrite the right item',
  updateLeft: 'Overwrite the left item',
  deleteRight: 'Delete on the right',
  deleteLeft: 'Delete on the left',
  conflict: 'Conflict: you decide',
};

export const actionVerb: Record<Action, string> = {
  none: '',
  createRight: 'Copying',
  createLeft: 'Copying',
  updateRight: 'Updating',
  updateLeft: 'Updating',
  deleteRight: 'Deleting',
  deleteLeft: 'Deleting',
  conflict: '',
};

/** Escapes text for safe use inside innerHTML. */
export function esc(s: string): string {
  return s.replace(/[&<>"']/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;', '"': '&quot;', "'": '&#39;' })[c]!);
}
