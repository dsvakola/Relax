// Relax - window "chrome": the menu bar, the saved-jobs pane, its splitter, and a small ask dialog.
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

import { invoke } from '@tauri-apps/api/core';
import { api } from './api';
import { esc } from './format';

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing element #${id}`);
  return node as T;
}

// ---------------------------------------------------------------------------
// Ask dialog (text input, or Yes / No / Cancel)
// ---------------------------------------------------------------------------

export interface AskOptions {
  title: string;
  text: string;
  /** Show a text box with this initial value. */
  input?: string;
  okLabel?: string;
  /** Show a third button (e.g. "Don't save"). */
  noLabel?: string;
  danger?: boolean;
  /** Return an error message to keep the dialog open. */
  validate?: (value: string) => string | null | Promise<string | null>;
}

export type AskResult = { button: 'ok' | 'no' | 'cancel'; value: string };

export function ask(o: AskOptions): Promise<AskResult> {
  el('askTitle').textContent = o.title;
  el('askText').textContent = o.text;
  const input = el<HTMLInputElement>('askInput');
  input.hidden = o.input === undefined;
  input.value = o.input ?? '';
  el('askError').hidden = true;
  const ok = el<HTMLButtonElement>('askOk');
  ok.textContent = o.okLabel ?? 'OK';
  ok.className = `btn ${o.danger ? 'btn-danger' : 'btn-primary'}`;
  const no = el<HTMLButtonElement>('askNo');
  no.hidden = !o.noLabel;
  no.textContent = o.noLabel ?? '';
  el('askModal').hidden = false;
  if (!input.hidden) {
    input.focus();
    input.select();
  } else ok.focus();

  return new Promise((resolve) => {
    const done = (button: AskResult['button']) => {
      el('askModal').hidden = true;
      input.onkeydown = null;
      resolve({ button, value: input.value.trim() });
    };
    ok.onclick = async () => {
      const err = o.validate ? await o.validate(input.value.trim()) : null;
      if (err) {
        el('askError').textContent = err;
        el('askError').hidden = false;
        return;
      }
      done('ok');
    };
    no.onclick = () => done('no');
    el('askCancel').onclick = () => done('cancel');
    input.onkeydown = (e) => {
      if (e.key === 'Enter') ok.click();
      if (e.key === 'Escape') done('cancel');
    };
  });
}

// ---------------------------------------------------------------------------
// Menu bar
// ---------------------------------------------------------------------------

export type Commands = Record<string, () => void>;

const menubar = () => el('menubar');

function closeMenus(): void {
  menubar()
    .querySelectorAll('.menu.open')
    .forEach((m) => m.classList.remove('open'));
}

function openMenu(menu: Element): void {
  closeMenus();
  menu.classList.add('open');
  menu.querySelector<HTMLButtonElement>('.menu-list button:not(:disabled)')?.focus();
}

export function initMenus(commands: Commands, isEnabled: (cmd: string) => boolean): void {
  const bar = menubar();
  bar.querySelectorAll('.menu').forEach((menu) => {
    const title = menu.querySelector<HTMLButtonElement>('.menu-title')!;
    title.addEventListener('mousedown', (e) => {
      e.preventDefault();
      if (menu.classList.contains('open')) closeMenus();
      else {
        refreshEnabled(menu, isEnabled);
        openMenu(menu);
      }
    });
    // Moving over another title while a menu is open switches menus, like Windows.
    title.addEventListener('mouseenter', () => {
      if (bar.querySelector('.menu.open') && !menu.classList.contains('open')) {
        refreshEnabled(menu, isEnabled);
        openMenu(menu);
      }
    });
    menu.querySelectorAll<HTMLButtonElement>('.menu-list button[data-cmd]').forEach((b) =>
      b.addEventListener('click', () => {
        closeMenus();
        bar.classList.remove('alt');
        commands[b.dataset.cmd!]?.();
      }),
    );
    // Arrow keys inside an open menu.
    menu.querySelector('.menu-list')!.addEventListener('keydown', (ev) => {
      const e = ev as KeyboardEvent;
      const items = [...menu.querySelectorAll<HTMLButtonElement>('.menu-list button:not(:disabled)')];
      const i = items.indexOf(document.activeElement as HTMLButtonElement);
      const menus = [...bar.querySelectorAll('.menu')];
      const mi = menus.indexOf(menu);
      if (e.key === 'ArrowDown') items[(i + 1) % items.length]?.focus();
      else if (e.key === 'ArrowUp') items[(i - 1 + items.length) % items.length]?.focus();
      else if (e.key === 'ArrowRight' || e.key === 'ArrowLeft') {
        const next = menus[(mi + (e.key === 'ArrowRight' ? 1 : menus.length - 1)) % menus.length];
        refreshEnabled(next, isEnabled);
        openMenu(next);
      } else if (e.key === 'Escape') closeMenus();
      else return;
      e.preventDefault();
      e.stopPropagation();
    });
  });
  document.addEventListener('mousedown', (e) => {
    if (!bar.contains(e.target as Node)) closeMenus();
  });
  window.addEventListener('blur', closeMenus);

  // Alt+F / Alt+E / Alt+T / Alt+H open the menus; holding Alt underlines the letters.
  const keys: Record<string, string> = { f: 'file', e: 'edit', t: 'tools', h: 'help' };
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Alt') bar.classList.add('alt');
    if (e.altKey && !e.ctrlKey && keys[e.key.toLowerCase()]) {
      const menu = bar.querySelector(`.menu[data-menu="${keys[e.key.toLowerCase()]}"]`);
      if (menu) {
        e.preventDefault();
        refreshEnabled(menu, isEnabled);
        openMenu(menu);
      }
    }
  });
  document.addEventListener('keyup', (e) => {
    if (e.key === 'Alt') bar.classList.remove('alt');
  });
}

function refreshEnabled(menu: Element, isEnabled: (cmd: string) => boolean): void {
  menu.querySelectorAll<HTMLButtonElement>('.menu-list button[data-cmd]').forEach((b) => {
    b.disabled = !isEnabled(b.dataset.cmd!);
  });
}

// ---------------------------------------------------------------------------
// Saved-jobs pane
// ---------------------------------------------------------------------------

interface JobEntry {
  path: string;
  name: string;
  modifiedMs: number;
}

export interface JobsPaneHooks {
  knownJobs: () => string[];
  currentJob: () => string;
  open: (path: string, thenCompare: boolean) => void;
  forget: (path: string) => void;
  renamed: (oldPath: string, newPath: string) => void;
  message: (title: string, text: string) => Promise<void>;
}

let hooks: JobsPaneHooks;
let menuJob: JobEntry | null = null;

export async function refreshJobs(): Promise<void> {
  if (!hooks) return;
  let jobs: JobEntry[] = [];
  try {
    jobs = await invoke<JobEntry[]>('list_jobs', { known: hooks.knownJobs() });
  } catch {
    // leave the list empty
  }
  const list = el('jobsList');
  const current = hooks.currentJob().toLowerCase();
  if (jobs.length === 0) {
    list.innerHTML =
      '<div class="jobs-empty">No saved jobs yet.<br>Choose your folders and settings, then click the save button above (Ctrl+S).</div>';
    return;
  }
  list.innerHTML = jobs
    .map(
      (j, i) => `<button class="job-item${j.path.toLowerCase() === current ? ' active' : ''}" data-i="${i}" title="${esc(j.path)}">
        <img src="/img/relax-icon.svg" alt=""><span class="job-name">${esc(j.name)}</span></button>`,
    )
    .join('');
  list.querySelectorAll<HTMLButtonElement>('.job-item').forEach((b) => {
    const job = jobs[Number(b.dataset.i)];
    b.addEventListener('click', () => hooks.open(job.path, false));
    b.addEventListener('dblclick', () => hooks.open(job.path, true));
    b.addEventListener('contextmenu', (e) => {
      e.preventDefault();
      menuJob = job;
      const m = el('jobMenu');
      m.hidden = false;
      m.style.left = `${Math.min(e.clientX, window.innerWidth - m.offsetWidth - 4)}px`;
      m.style.top = `${Math.min(e.clientY, window.innerHeight - m.offsetHeight - 4)}px`;
    });
  });
}

export function initJobsPane(h: JobsPaneHooks): void {
  hooks = h;
  const m = el('jobMenu');
  document.addEventListener('mousedown', (e) => {
    if (!m.hidden && !m.contains(e.target as Node)) m.hidden = true;
  });
  m.querySelectorAll<HTMLButtonElement>('[data-job]').forEach((b) =>
    b.addEventListener('click', async () => {
      m.hidden = true;
      const job = menuJob;
      if (!job) return;
      switch (b.dataset.job) {
        case 'open':
          hooks.open(job.path, false);
          break;
        case 'compare':
          hooks.open(job.path, true);
          break;
        case 'reveal':
          await api.reveal(job.path).catch(() => {});
          break;
        case 'remove':
          hooks.forget(job.path);
          await refreshJobs();
          break;
        case 'rename': {
          const r = await ask({
            title: 'Rename job',
            text: `New name for "${job.name}":`,
            input: job.name,
            okLabel: 'Rename',
            validate: (v) => (v ? null : 'Please enter a name.'),
          });
          if (r.button !== 'ok' || r.value === job.name) break;
          try {
            const res = await invoke<{ path: string; scheduleRemoved: boolean }>('rename_job_file', {
              path: job.path,
              newName: r.value,
            });
            hooks.renamed(job.path, res.path);
            if (res.scheduleRemoved)
              await hooks.message(
                'Schedule removed',
                'This job was scheduled to run automatically. Because its file name changed, please set the schedule again (Tools → Schedule this job).',
              );
          } catch (e) {
            await hooks.message('Cannot rename', String(e));
          }
          await refreshJobs();
          break;
        }
        case 'delete': {
          const r = await ask({
            title: 'Delete job',
            text: `Delete the job "${job.name}"?\n\nThe job file goes to the Recycle Bin, and its schedule (if any) is removed. Your backed-up files are not touched.`,
            okLabel: 'Delete job',
            danger: true,
          });
          if (r.button !== 'ok') break;
          try {
            await invoke('delete_job_file', { path: job.path });
            hooks.forget(job.path);
          } catch (e) {
            await hooks.message('Cannot delete', String(e));
          }
          await refreshJobs();
          break;
        }
      }
    }),
  );
  initSplitter();
}

// ---------------------------------------------------------------------------
// Pane width and visibility (remembered per user)
// ---------------------------------------------------------------------------

const PANE_KEY = 'relax.pane.v1';

function loadPane(): { width: number; hidden: boolean } {
  try {
    const p = JSON.parse(localStorage.getItem(PANE_KEY) ?? '{}');
    return { width: Number(p.width) || 230, hidden: !!p.hidden };
  } catch {
    return { width: 230, hidden: false };
  }
}

function savePane(p: { width: number; hidden: boolean }): void {
  try {
    localStorage.setItem(PANE_KEY, JSON.stringify(p));
  } catch {
    // not remembered
  }
}

let pane = loadPane();

function applyPane(): void {
  el('jobsPane').style.width = `${pane.width}px`;
  el('workspace').classList.toggle('pane-hidden', pane.hidden);
  el('menuPaneLabel').textContent = pane.hidden ? 'Show saved jobs pane' : 'Hide saved jobs pane';
  window.dispatchEvent(new Event('resize'));
}

export function togglePane(): void {
  pane = { ...pane, hidden: !pane.hidden };
  // On a narrow window the pane is normally hidden; showing it by hand keeps it visible.
  el('workspace').classList.toggle('pane-forced', !pane.hidden);
  savePane(pane);
  applyPane();
}

function initSplitter(): void {
  applyPane();
  const splitter = el('paneSplitter');
  splitter.addEventListener('mousedown', (e) => {
    e.preventDefault();
    splitter.classList.add('dragging');
    const startX = e.clientX;
    const startW = el('jobsPane').offsetWidth;
    const move = (ev: MouseEvent) => {
      pane.width = Math.min(420, Math.max(160, startW + ev.clientX - startX));
      el('jobsPane').style.width = `${pane.width}px`;
      window.dispatchEvent(new Event('resize'));
    };
    const up = () => {
      splitter.classList.remove('dragging');
      document.removeEventListener('mousemove', move);
      document.removeEventListener('mouseup', up);
      savePane(pane);
    };
    document.addEventListener('mousemove', move);
    document.addEventListener('mouseup', up);
  });
}
