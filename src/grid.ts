// Relax - virtual preview grid. Only the visible rows exist in the page, so even 500,000 items
// scroll smoothly. Rows are fetched from Rust in blocks as the user scrolls.

import { api, type Action, type GridRow } from './api';
import { actionLabel, esc, formatBytes, formatDate } from './format';

const ROW_H = 26;
const BLOCK = 200;

const FOLDER_ICON =
  '<svg class="ico ico-folder" viewBox="0 0 24 24"><path d="M3 6a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v10a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2z"/></svg>';
const FILE_ICON =
  '<svg class="ico ico-file" viewBox="0 0 24 24"><path d="M14 3H6a1 1 0 0 0-1 1v16a1 1 0 0 0 1 1h12a1 1 0 0 0 1-1V8z"/><path d="M14 3v5h5"/></svg>';

/** Small symbol shown in the middle "Action" column. */
function actionIcon(a: Action, equal: boolean): string {
  const map: Record<Action, [string, string]> = {
    none: equal ? ['=', 'act-equal'] : ['–', 'act-skip'],
    createRight: ['+→', 'act-create'],
    createLeft: ['←+', 'act-create'],
    updateRight: ['→', 'act-update'],
    updateLeft: ['←', 'act-update'],
    deleteRight: ['✕→', 'act-delete'],
    deleteLeft: ['←✕', 'act-delete'],
    conflict: ['⚠', 'act-conflict'],
  };
  const [sym, cls] = map[a];
  return `<span class="act ${cls}">${sym}</span>`;
}

export class VirtualGrid {
  private body: HTMLElement;
  private spacer: HTMLElement;
  private layer: HTMLElement;
  private rowCount = 0;
  private blocks = new Map<number, GridRow[]>();
  private loading = new Set<number>();
  private generation = 0;
  private selected = new Set<number>();
  /** "Select all": every visible row, without loading all of them. */
  private allSelected = false;
  private anchor: number | null = null;
  private rafPending = false;

  /** Called when the user right-clicks; receives the selected row ids. */
  onContextMenu: (ids: number[], rows: GridRow[], x: number, y: number) => void = () => {};

  selectAll(): void {
    if (this.rowCount === 0) return;
    this.allSelected = true;
    this.selected.clear();
    this.schedule();
  }

  /** Current selection: ids, or `all` when everything visible is selected. */
  selection(): { ids: number[]; all: boolean } {
    return { ids: [...this.selected], all: this.allSelected };
  }

  hasSelection(): boolean {
    return this.allSelected || this.selected.size > 0;
  }

  constructor(body: HTMLElement) {
    this.body = body;
    this.spacer = document.createElement('div');
    this.spacer.className = 'grid-spacer';
    this.layer = document.createElement('div');
    this.layer.className = 'grid-layer';
    this.body.append(this.spacer, this.layer);
    this.body.addEventListener('scroll', () => this.schedule());
    new ResizeObserver(() => this.schedule()).observe(this.body);
    this.layer.addEventListener('mousedown', (e) => this.onMouseDown(e));
    this.layer.addEventListener('contextmenu', (e) => this.onRightClick(e));
  }

  /** Shows a new set of rows (after Compare or a view change). */
  reset(rowCount: number, keepScroll = false): void {
    this.generation++;
    this.rowCount = rowCount;
    this.blocks.clear();
    this.loading.clear();
    if (!keepScroll) {
      this.allSelected = false;
      this.selected.clear();
      this.anchor = null;
      this.body.scrollTop = 0;
    }
    this.spacer.style.height = `${rowCount * ROW_H}px`;
    this.body.classList.toggle('has-rows', rowCount > 0);
    this.schedule();
  }

  clear(): void {
    this.reset(0);
  }

  private schedule(): void {
    if (this.rafPending) return;
    this.rafPending = true;
    requestAnimationFrame(() => {
      this.rafPending = false;
      this.render();
    });
  }

  private visibleRange(): [number, number] {
    const first = Math.max(0, Math.floor(this.body.scrollTop / ROW_H) - 10);
    const last = Math.min(this.rowCount, Math.ceil((this.body.scrollTop + this.body.clientHeight) / ROW_H) + 10);
    return [first, last];
  }

  private rowAt(i: number): GridRow | undefined {
    return this.blocks.get(Math.floor(i / BLOCK))?.[i % BLOCK];
  }

  private ensureBlock(b: number): void {
    if (this.blocks.has(b) || this.loading.has(b)) return;
    this.loading.add(b);
    const gen = this.generation;
    api
      .getRows(b * BLOCK, BLOCK)
      .then((rows) => {
        if (gen !== this.generation) return;
        this.blocks.set(b, rows);
        this.loading.delete(b);
        this.schedule();
      })
      .catch(() => this.loading.delete(b));
  }

  private render(): void {
    const [first, last] = this.visibleRange();
    for (let b = Math.floor(first / BLOCK); b <= Math.floor(Math.max(first, last - 1) / BLOCK); b++) {
      this.ensureBlock(b);
    }
    let html = '';
    for (let i = first; i < last; i++) {
      const r = this.rowAt(i);
      const top = i * ROW_H;
      if (!r) {
        html += `<div class="grow grow-loading" style="top:${top}px"></div>`;
        continue;
      }
      if (r.header) {
        html += `<div class="grow grow-header${r.note ? ' grow-blocked' : ''}" style="top:${top}px" data-i="${i}">
          <span>${esc(r.path)}</span>${r.note ? `<span class="blocked-note">Not synchronized: ${esc(r.note.split('\n')[0])}</span>` : ''}</div>`;
        continue;
      }
      const sel = this.allSelected || this.selected.has(r.id) ? ' selected' : '';
      const icon = r.kind === 'folder' ? FOLDER_ICON : FILE_ICON;
      const indent = `style="padding-left:${8 + r.depth * 16}px"`;
      const side = (c: GridRow['left']) =>
        c
          ? `<div class="gc gname" ${indent} title="${esc(r.path)}">${icon}<span class="nm">${esc(c.name)}</span></div>
             <div class="gc gsize">${r.kind === 'folder' ? '' : formatBytes(c.size)}</div>
             <div class="gc gdate">${r.kind === 'folder' ? '' : formatDate(c.modifiedMs)}</div>`
          : `<div class="gc gname gempty"></div><div class="gc gsize gempty"></div><div class="gc gdate gempty"></div>`;
      const tip = esc(`${actionLabel[r.action!]}${r.note ? `\n${r.note}` : ''}`);
      html += `<div class="grow cat-${r.category} act-row-${r.action}${sel}" style="top:${top}px" data-i="${i}">
        ${side(r.left)}<div class="gc gact" title="${tip}">${actionIcon(r.action!, r.category === 'equal')}</div>${side(r.right)}</div>`;
    }
    this.layer.innerHTML = html;
  }

  private indexFromEvent(e: MouseEvent): number | null {
    const row = (e.target as HTMLElement).closest<HTMLElement>('.grow');
    if (!row || row.dataset.i === undefined) return null;
    return Number(row.dataset.i);
  }

  private onMouseDown(e: MouseEvent): void {
    if (e.button !== 0) return;
    const i = this.indexFromEvent(e);
    if (i === null) return;
    const r = this.rowAt(i);
    if (!r || r.header) return;
    this.allSelected = false;
    if (e.shiftKey && this.anchor !== null) {
      this.selected.clear();
      const [a, b] = [Math.min(this.anchor, i), Math.max(this.anchor, i)];
      for (let k = a; k <= b; k++) {
        const rk = this.rowAt(k);
        if (rk && !rk.header) this.selected.add(rk.id);
      }
    } else if (e.ctrlKey) {
      if (this.selected.has(r.id)) this.selected.delete(r.id);
      else this.selected.add(r.id);
      this.anchor = i;
    } else {
      this.selected.clear();
      this.selected.add(r.id);
      this.anchor = i;
    }
    this.schedule();
  }

  private onRightClick(e: MouseEvent): void {
    e.preventDefault();
    const i = this.indexFromEvent(e);
    if (i === null) return;
    const r = this.rowAt(i);
    if (!r || r.header) return;
    if (!this.allSelected && !this.selected.has(r.id)) {
      this.selected.clear();
      this.selected.add(r.id);
      this.anchor = i;
      this.render();
    }
    const rows: GridRow[] = [];
    for (const rows_ of this.blocks.values()) {
      for (const x of rows_) if (this.selected.has(x.id)) rows.push(x);
    }
    this.onContextMenu([...this.selected], rows, e.clientX, e.clientY);
  }
}
