// Relax - the Schedule window (Windows Task Scheduler) and the Logs window.
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.

import { api, type Schedule } from './api';
import { esc } from './format';
import { jobName, prefs, saveJob } from './settings';

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing element #${id}`);
  return node as T;
}

type Message = (title: string, text: string) => Promise<void>;

// ---------------------------------------------------------------------------
// Schedule
// ---------------------------------------------------------------------------

/** Shows a green dot on the Schedule button when the current job runs automatically. */
export async function refreshScheduleDot(): Promise<void> {
  if (!prefs.jobPath) {
    el('scheduleDot').hidden = true;
    return;
  }
  try {
    const info = await api.getSchedule(prefs.jobPath);
    el('scheduleDot').hidden = !info.exists;
  } catch {
    el('scheduleDot').hidden = true;
  }
}

function showScheduleStatus(exists: boolean, nextRun: string): void {
  el('schedStatus').textContent = exists
    ? `This job is scheduled.${nextRun ? ` Next run: ${nextRun}` : ''}`
    : 'This job is not scheduled yet.';
  el<HTMLButtonElement>('btnSchedRemove').disabled = !exists;
}

export async function openSchedule(message: Message): Promise<void> {
  if (!prefs.jobPath) {
    await message(
      'Save the job first',
      'A scheduled run uses a saved job file. Choose where to save this job, then open Schedule again.',
    );
    if (!(await saveJob())) return;
  }
  const path = prefs.jobPath;
  el('schedJob').textContent = `Job: ${jobName()}  (${path})`;
  el('schedStatus').textContent = 'Checking…';
  el('scheduleModal').hidden = false;
  try {
    const info = await api.getSchedule(path);
    showScheduleStatus(info.exists, info.nextRun);
  } catch (e) {
    el('schedStatus').textContent = String(e);
  }

  el('btnSchedCancel').onclick = () => (el('scheduleModal').hidden = true);
  el('btnSchedRemove').onclick = async () => {
    try {
      await api.deleteSchedule(path);
      showScheduleStatus(false, '');
      void refreshScheduleDot();
    } catch (e) {
      await message('Cannot remove schedule', String(e));
    }
  };
  el('btnSchedSave').onclick = async () => {
    // Save the current settings first, so the scheduled run does exactly what is on screen.
    await saveJob();
    const kind = (document.querySelector<HTMLInputElement>('input[name="schedKind"]:checked')?.value ??
      'daily') as Schedule['kind'];
    const schedule: Schedule = {
      kind,
      time: el<HTMLInputElement>('schedTime').value || '21:00',
      day: el<HTMLSelectElement>('schedDay').value,
    };
    try {
      const info = await api.createSchedule(path, schedule);
      showScheduleStatus(info.exists, info.nextRun);
      void refreshScheduleDot();
    } catch (e) {
      await message('Cannot create schedule', String(e));
    }
  };
}

// ---------------------------------------------------------------------------
// Logs
// ---------------------------------------------------------------------------

async function showLog(name: string): Promise<void> {
  el('logsList')
    .querySelectorAll<HTMLButtonElement>('button')
    .forEach((b) => b.classList.toggle('on', b.dataset.name === name));
  try {
    el('logText').textContent = await api.readLog(name);
  } catch (e) {
    el('logText').textContent = String(e);
  }
}

/** Opens the Logs window, optionally selecting a given log file (full path or name). */
export async function openLogs(select?: string): Promise<void> {
  const names = await api.listLogs();
  const list = el('logsList');
  list.innerHTML = names.length
    ? names
        .map((n) => {
          const m = /^(\d{4}-\d{2}-\d{2}) (\d{2})(\d{2})(\d{2})(.*) - (\w+)\.txt$/.exec(n);
          const label = m
            ? `${m[1]} ${m[2]}:${m[3]}${m[5] ? ` · ${esc(m[5].trim())}` : ''}<br><span class="st-${m[6]}">${m[6]}</span>`
            : esc(n);
          return `<button data-name="${esc(n)}">${label}</button>`;
        })
        .join('')
    : '<p class="hint" style="padding:12px">No syncs yet.</p>';
  el('logText').textContent = names.length ? '' : 'The log of every sync will appear here.';
  list.querySelectorAll<HTMLButtonElement>('button').forEach((b) => (b.onclick = () => void showLog(b.dataset.name!)));
  el('btnLogsFolder').onclick = () => void api.openLogsFolder();
  el('btnLogsClose').onclick = () => (el('logsModal').hidden = true);
  el('logsModal').hidden = false;
  const wanted = select ? select.split(/[\\/]/).pop()! : names[0];
  if (wanted) await showLog(wanted);
}
