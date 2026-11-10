// Relax - Microsoft Store trial badge and "Buy Relax" screen.
// Developed by Prof. Dattaraj S. Vidyasagar & Prof. Yash D. Vidyasagar, for Vidyasagar Academy.
//
// Only the Microsoft Store version has a trial: other versions report "free" and show nothing here.
// The Buy screen never touches the user's files, jobs or backups.

import { getCurrentWindow } from '@tauri-apps/api/window';
import { api, type LicenseInfo } from './api';

function el<T extends HTMLElement = HTMLElement>(id: string): T {
  const node = document.getElementById(id);
  if (!node) throw new Error(`Missing element #${id}`);
  return node as T;
}

let busy = false;

function apply(info: LicenseInfo): void {
  const badge = el('trialBadge');
  const buyBtn = el('btnBuyTrial');
  const screen = el('buyModal');

  const trial = info.state === 'trial';
  badge.hidden = !trial;
  buyBtn.hidden = !trial;
  if (trial) {
    const d = info.daysLeft ?? 0;
    badge.textContent = `Trial: ${d} ${d === 1 ? 'day' : 'days'} left`;
  }

  const expired = info.state === 'expired';
  screen.hidden = !expired;
  if (expired) {
    el('buyPrice').textContent = info.price;
    el('btnBuyNow').focus();
  }
}

export async function refreshLicense(): Promise<void> {
  try {
    apply(await api.getLicense());
  } catch {
    // The licence can never block the user: if it cannot be read, show nothing.
    apply({ state: 'unknown', daysLeft: null, price: '' });
  }
}

async function buy(): Promise<void> {
  if (busy) return;
  busy = true;
  const err = el('buyError');
  err.hidden = true;
  try {
    apply(await api.buyApp());
  } catch (e) {
    err.textContent = String(e instanceof Error ? e.message : e);
    err.hidden = false;
  } finally {
    busy = false;
  }
}

export function initLicense(): void {
  el('btnBuyTrial').onclick = () => void buy();
  el('btnBuyNow').onclick = () => void buy();
  el('btnBuyCheck').onclick = () => {
    el('buyError').hidden = true;
    void refreshLicense();
  };
  el('btnBuyExit').onclick = () => void getCurrentWindow().destroy();

  // While the Buy screen is open, keyboard shortcuts must not reach the screens behind it.
  window.addEventListener(
    'keydown',
    (e) => {
      if (el('buyModal').hidden) return;
      if (e.key === 'Tab' || e.key === 'Enter' || e.key === ' ') return; // keep the screen's own buttons usable
      e.stopPropagation();
      e.preventDefault();
    },
    true,
  );

  // After buying in the Store (or when the trial runs out while Relax is open), the next click into the window rechecks.
  window.addEventListener('focus', () => void refreshLicense());
  void refreshLicense();
}
