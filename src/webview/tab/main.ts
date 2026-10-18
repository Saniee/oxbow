// The SlothGit tab's page (see tab/tabPanel.ts). Everything is built with DOM APIs, never innerHTML, so commit
// messages and file names can't inject markup. Rows carry data-vscode-context, so right-clicking one opens VS Code's
// own menu (package.json webview/context) with the row's commands.
import '@vscode/codicons/dist/codicon.css';
import type { LogEntry } from '../../git/commandLog';
import type { DisplayFile } from '../../git/diff';
import type { GraphRef } from '../../git/graph';
import type { DetailBlock, FromTab, ListId, TabAction, TabCommands, TabDetail, TabList, TabRow, TabStatus, ToTab } from '../messages';
import './style.css';

const vscode = acquireVsCodeApi<FromTab>();

// --- persisted page state ---------------------------------------------------------------------

interface PageState {
  leftWidth?: number;
  logHeight?: number;
  /** The list shown in each panel. */
  tabs: Record<string, ListId>;
  focused: string;
  draft: string;
  amend: boolean;
  /** Remotes whose branches are shown. */
  expanded: string[];
}

const state: PageState = { tabs: {}, focused: 'commits', draft: '', amend: false, expanded: [], ...(vscode.getState() as Partial<PageState> | undefined) };
const save = () => vscode.setState(state);

// --- helpers ------------------------------------------------------------------------------------

function h<K extends keyof HTMLElementTagNameMap>(tag: K, cls?: string, text?: string): HTMLElementTagNameMap[K] {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text !== undefined) e.textContent = text;
  return e;
}

function icon(id: string, color?: string): HTMLSpanElement {
  const e = h('span', `codicon codicon-${id}`);
  if (color) e.style.color = color;
  return e;
}

function iconButton(id: string, title: string, onClick: (e: MouseEvent) => void): HTMLButtonElement {
  const b = h('button', `icon-button codicon codicon-${id}`);
  b.title = title;
  b.setAttribute('aria-label', title);
  b.addEventListener('click', (e) => {
    e.stopPropagation();
    onClick(e);
  });
  // Clicking a row button must not select the row (that happens on mousedown).
  b.addEventListener('mousedown', (e) => e.stopPropagation());
  return b;
}

const $ = <T extends HTMLElement>(id: string) => document.getElementById(id) as T;
let commands: TabCommands = {};
const titleOf = (command: string) => commands[command]?.title ?? command;
const run = (command: string, id?: string) => vscode.postMessage({ type: 'command', command, id });

// --- the ⋯ menu ---------------------------------------------------------------------------------

const menu = $<HTMLDivElement>('menu');

function showMenu(anchor: HTMLElement, items: { label: string; icon?: string; run: () => void }[]): void {
  menu.textContent = '';
  for (const item of items) {
    const b = h('button');
    b.setAttribute('role', 'menuitem');
    b.append(item.icon ? icon(item.icon) : h('span', 'codicon'), h('span', '', item.label));
    b.addEventListener('click', () => {
      hideMenu();
      item.run();
    });
    menu.appendChild(b);
  }
  menu.classList.remove('hidden');
  const rect = anchor.getBoundingClientRect();
  const width = menu.offsetWidth;
  const height = menu.offsetHeight;
  menu.style.left = `${Math.max(4, Math.min(rect.right - width, window.innerWidth - width - 4))}px`;
  menu.style.top = `${rect.bottom + height + 4 > window.innerHeight ? Math.max(4, rect.top - height - 2) : rect.bottom + 2}px`;
  (menu.firstElementChild as HTMLElement | null)?.focus();
}

function hideMenu(): void {
  menu.classList.add('hidden');
}

document.addEventListener('mousedown', (e) => {
  if (!menu.contains(e.target as Node)) hideMenu();
});
window.addEventListener('blur', hideMenu);
menu.addEventListener('keydown', (e) => e.key === 'Escape' && hideMenu());

/** Header buttons: the primary ones as icons, the rest in a ⋯ menu. */
function renderActions(container: HTMLElement, actions: TabAction[]): void {
  container.textContent = '';
  for (const a of actions.filter((x) => x.primary)) {
    container.appendChild(iconButton(commands[a.command]?.icon ?? 'circle-large-outline', titleOf(a.command), () => run(a.command)));
  }
  const more = actions.filter((x) => !x.primary);
  if (more.length) {
    const b = iconButton('ellipsis', 'More Actions…', () =>
      showMenu(b, more.map((a) => ({ label: titleOf(a.command), icon: commands[a.command]?.icon, run: () => run(a.command) }))),
    );
    container.appendChild(b);
  }
}

// --- panels -------------------------------------------------------------------------------------

interface PanelDef {
  id: string;
  title: string;
  lists: { id: ListId; label: string; advanced?: boolean }[];
}

const PANELS: PanelDef[] = [
  { id: 'files', title: 'Files', lists: [{ id: 'files', label: 'Files' }] },
  {
    id: 'branches',
    title: 'Branches',
    lists: [
      { id: 'local', label: 'Local' },
      { id: 'remotes', label: 'Remotes' },
      { id: 'tags', label: 'Tags' },
      { id: 'worktrees', label: 'Worktrees', advanced: true },
    ],
  },
  {
    id: 'commits',
    title: 'Commits',
    lists: [
      { id: 'commits', label: 'Commits' },
      { id: 'reflog', label: 'Reflog', advanced: true },
    ],
  },
  { id: 'stash', title: 'Stash', lists: [{ id: 'stash', label: 'Stash' }] },
];

interface PanelEls {
  def: PanelDef;
  section: HTMLElement;
  tabs: Map<ListId, HTMLButtonElement>;
  count: HTMLSpanElement;
  description: HTMLSpanElement;
  actions: HTMLSpanElement;
  bodies: Map<ListId, HTMLDivElement>;
}

const left = $<HTMLDivElement>('left');
const panels = new Map<string, PanelEls>();
const panelOfList = new Map<ListId, PanelEls>();
const lists = new Map<ListId, TabList>();
const selections = new Map<ListId, string[]>();
/** Where a shift-click range starts. */
const anchors = new Map<ListId, string>();
let simple = false;

function activeList(panel: PanelEls): ListId {
  const wanted = state.tabs[panel.def.id];
  const available = panel.def.lists.filter((l) => !(simple && l.advanced));
  return available.find((l) => l.id === wanted)?.id ?? available[0].id;
}

function focusPanel(id: string): void {
  state.focused = id;
  save();
  for (const [pid, p] of panels) p.section.classList.toggle('focused', pid === id);
  status.section.classList.toggle('focused', id === 'status');
}

function switchTab(panel: PanelEls, list: ListId): void {
  state.tabs[panel.def.id] = list;
  save();
  updatePanel(panel);
}

function updatePanel(panel: PanelEls): void {
  const active = activeList(panel);
  for (const l of panel.def.lists) {
    panel.tabs.get(l.id)?.classList.toggle('active', l.id === active);
    panel.tabs.get(l.id)?.classList.toggle('hidden', simple && !!l.advanced);
    panel.bodies.get(l.id)!.classList.toggle('hidden', l.id !== active);
  }
  const list = lists.get(active);
  const count = list ? String(list.rows.filter((r) => !r.parent).length) : '';
  // In a panel with tabs, the count goes in the active tab, so it doesn't read as the last tab's.
  if (panel.tabs.size) {
    for (const l of panel.def.lists) {
      const tab = panel.tabs.get(l.id)!;
      tab.textContent = l.label;
      if (l.id === active && count) tab.appendChild(h('span', 'count', count));
    }
    panel.count.textContent = '';
  } else {
    panel.count.textContent = count;
  }
  panel.description.textContent = list?.description ?? '';
  panel.description.title = list?.description ?? '';
  renderActions(panel.actions, list?.actions ?? []);
}

function buildPanel(def: PanelDef): PanelEls {
  const section = h('section', 'panel list-panel');
  section.dataset.panel = def.id;
  const header = h('header');
  const tabs = new Map<ListId, HTMLButtonElement>();
  if (def.lists.length > 1) {
    const strip = h('span', 'tabs');
    for (const l of def.lists) {
      const t = h('button', 'tab', l.label);
      t.addEventListener('click', () => {
        switchTab(panels.get(def.id)!, l.id);
        focusPanel(def.id);
      });
      tabs.set(l.id, t);
      strip.appendChild(t);
    }
    header.appendChild(strip);
  } else {
    header.appendChild(h('span', 'title', def.title));
  }
  const count = h('span', 'count');
  const description = h('span', 'description');
  const actions = h('span', 'actions');
  header.append(count, description, h('span', 'spacer'), actions);
  section.appendChild(header);
  const bodies = new Map<ListId, HTMLDivElement>();
  const body = h('div', 'body');
  for (const l of def.lists) {
    const list = h('div', 'list');
    list.dataset.list = l.id;
    attachListEvents(list, l.id);
    bodies.set(l.id, list);
    body.appendChild(list);
  }
  section.appendChild(body);
  section.addEventListener('mousedown', () => focusPanel(def.id), true);
  const panel: PanelEls = { def, section, tabs, count, description, actions, bodies };
  for (const l of def.lists) panelOfList.set(l.id, panel);
  return panel;
}

// --- rows ---------------------------------------------------------------------------------------

function labelWithHighlights(row: TabRow): HTMLSpanElement {
  const label = h('span', 'label');
  if (!row.highlights?.length) {
    label.textContent = row.label;
    return label;
  }
  let at = 0;
  for (const [start, end] of row.highlights) {
    if (start > at) label.append(row.label.slice(at, start));
    label.appendChild(h('mark', '', row.label.slice(start, end)));
    at = end;
  }
  label.append(row.label.slice(at));
  return label;
}

function pills(refs: GraphRef[]): HTMLSpanElement {
  const kinds: Record<GraphRef['kind'], string> = { head: 'Checked out', local: 'Branch', remote: 'Remote branch', tag: 'Tag' };
  const span = h('span', 'pills');
  for (const ref of refs) {
    const pill = h('span', `pill ${ref.kind}`, ref.name);
    pill.title = `${kinds[ref.kind]}: ${ref.name}`;
    span.appendChild(pill);
  }
  return span;
}

function statusLetters(status: string): HTMLSpanElement {
  const conflict = /^(DD|AU|UD|UA|DU|AA|UU)$/.test(status);
  const span = h('span', `status-letters${conflict ? ' conflict' : ''}`);
  span.append(h('span', 'x', status[0]), h('span', `y${status === '??' ? ' untracked' : ''}`, status[1]));
  return span;
}

function rowElement(row: TabRow, section: string, expanded?: boolean): HTMLDivElement {
  const el = h('div', 'row');
  el.dataset.id = row.id;
  // VS Code's webview context menu reads this (package.json webview/context).
  el.dataset.vscodeContext = JSON.stringify({
    webviewSection: section,
    slothgitView: row.view,
    slothgitItem: row.item,
    slothgitId: row.id,
    preventDefaultContextMenuItems: true,
  });
  if (row.dim) el.classList.add('dim');
  if (/:head\b/.test(row.item)) el.classList.add('head');
  if (row.parent) el.classList.add('child');
  if (row.expandable) {
    const twistie = h('span', `twistie codicon codicon-chevron-${expanded ? 'down' : 'right'}`);
    twistie.dataset.toggle = row.id;
    el.appendChild(twistie);
  }
  if (row.icon) el.appendChild(icon(row.icon.id, row.icon.color));

  if (row.commit) {
    const c = row.commit;
    const sha = h('span', 'sha', c.shortSha);
    if (row.icon?.color) sha.style.color = row.icon.color;
    el.appendChild(sha);
    if (c.people) el.appendChild(h('span', 'people', c.people));
    if (c.refs.length) el.appendChild(pills(c.refs));
    el.appendChild(h('span', 'subject', c.subject));
    el.appendChild(h('span', 'date', c.date));
  } else {
    if (row.status && row.status.length === 2) el.appendChild(statusLetters(row.status));
    else if (row.status) el.appendChild(h('span', `file-status st-${row.status}`, row.status));
    el.appendChild(labelWithHighlights(row));
    if (row.description) el.appendChild(h('span', 'desc', row.description));
  }
  if (row.tooltip) el.title = row.tooltip;

  if (row.actions.length) {
    const actions = h('span', 'row-actions');
    for (const command of row.actions) {
      actions.appendChild(iconButton(commands[command]?.icon ?? 'circle-large-outline', titleOf(command), () => run(command, row.id)));
    }
    el.appendChild(actions);
  }
  return el;
}

function renderList(id: ListId): void {
  const data = lists.get(id);
  const panel = panelOfList.get(id);
  if (!data || !panel) return;
  const container = panel.bodies.get(id)!;
  const scroll = container.parentElement!.scrollTop;
  container.textContent = '';
  const selected = new Set(selections.get(id) ?? []);
  if (!data.rows.length) container.appendChild(h('div', 'empty', data.empty));
  for (const row of data.rows) {
    if (row.parent && !state.expanded.includes(row.parent)) continue;
    const el = rowElement(row, id, state.expanded.includes(row.id));
    if (selected.has(row.id)) el.classList.add('selected');
    container.appendChild(el);
  }
  container.parentElement!.scrollTop = scroll;
  if (panel === panelOfList.get(id) && activeList(panel) === id) updatePanel(panel);
}

function paintSelection(id: ListId): void {
  const container = panelOfList.get(id)?.bodies.get(id);
  if (!container) return;
  const selected = new Set(selections.get(id) ?? []);
  for (const row of container.querySelectorAll<HTMLElement>('.row')) row.classList.toggle('selected', selected.has(row.dataset.id!));
}

function select(list: ListId, ids: string[], notify = true): void {
  selections.set(list, ids);
  paintSelection(list);
  if (notify) vscode.postMessage({ type: 'select', list, ids });
}

function attachListEvents(container: HTMLDivElement, list: ListId): void {
  container.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    const target = e.target as HTMLElement;
    const toggle = target.closest<HTMLElement>('[data-toggle]')?.dataset.toggle;
    if (toggle) {
      state.expanded = state.expanded.includes(toggle) ? state.expanded.filter((x) => x !== toggle) : [...state.expanded, toggle];
      save();
      renderList(list);
      return;
    }
    const id = target.closest<HTMLElement>('.row')?.dataset.id;
    if (!id) return;
    const current = selections.get(list) ?? [];
    if (e.ctrlKey || e.metaKey) {
      select(list, current.includes(id) ? current.filter((x) => x !== id) : [id, ...current]);
      anchors.set(list, id);
    } else if (e.shiftKey && anchors.get(list)) {
      const visible = [...container.querySelectorAll<HTMLElement>('.row')].map((r) => r.dataset.id!);
      const [a, b] = [visible.indexOf(anchors.get(list)!), visible.indexOf(id)].sort((x, y) => x - y);
      const range = visible.slice(a, b + 1);
      // The clicked row first: the detail pane follows it.
      select(list, [id, ...range.filter((x) => x !== id)]);
    } else {
      anchors.set(list, id);
      if (current.length !== 1 || current[0] !== id) select(list, [id]);
    }
  });
  // Right-click selects the row first, as in a tree, unless it is part of the selection.
  container.addEventListener('contextmenu', (e) => {
    const id = (e.target as HTMLElement).closest<HTMLElement>('.row')?.dataset.id;
    if (id && !(selections.get(list) ?? []).includes(id)) select(list, [id]);
  });
  container.addEventListener('dblclick', (e) => {
    const target = e.target as HTMLElement;
    if (target.closest('.row-actions, [data-toggle]')) return;
    const id = target.closest<HTMLElement>('.row')?.dataset.id;
    if (id) vscode.postMessage({ type: 'open', id });
  });
}

// --- status panel -------------------------------------------------------------------------------

const status = (() => {
  const section = h('section', 'panel status');
  section.dataset.panel = 'status';
  const header = h('header');
  const actions = h('span', 'actions');
  header.append(h('span', 'title', 'Status'), h('span', 'spacer'), actions);
  const body = h('div', 'status-body');
  section.append(header, body);
  section.addEventListener('mousedown', () => focusPanel('status'), true);
  return { section, actions, body };
})();

function splitButton(menu: 'pull' | 'push', label: string, title: string, command: string, enabled: boolean, primary: boolean): HTMLElement {
  const split = h('div', `split${primary ? ' primary' : ''}`);
  // Right-clicking the button opens the same menu natively (package.json webview/context).
  split.dataset.vscodeContext = JSON.stringify({ webviewSection: menu, preventDefaultContextMenuItems: true });
  const main = h('button', 'main');
  main.append(icon(menu === 'pull' ? 'arrow-down' : 'arrow-up'), h('span', '', label));
  main.title = title;
  main.disabled = !enabled;
  main.addEventListener('click', () => run(command));
  const more = h('button', 'more');
  more.appendChild(icon('chevron-down'));
  more.title = menu === 'pull' ? 'More Pull Actions' : 'More Push Actions';
  more.addEventListener('click', () => vscode.postMessage({ type: 'pullPush', menu }));
  split.append(main, more);
  return split;
}

function renderStatus(s: TabStatus): void {
  if (s.simple !== simple) {
    simple = s.simple;
    for (const p of panels.values()) updatePanel(p);
  }
  renderActions(status.actions, s.actions);
  const body = status.body;
  body.textContent = '';
  const where = h('div', 'where');
  if (!s.repo) {
    where.append(icon('info'), h('span', '', 'No Git repository is open.'));
    body.appendChild(where);
    return;
  }
  where.append(icon('repo'), h('span', 'repo', s.repo), icon('chevron-right'));
  const b = s.branch;
  if (b) {
    const branch = h('span', 'branch', b.name);
    branch.title = b.name;
    where.append(icon('git-branch'), branch);
    if (b.upstream && !b.gone) where.appendChild(h('span', 'counts', `↑${b.ahead} ↓${b.behind}`));
    const upstream = h('span', 'upstream', !b.upstream ? (s.hasRemotes ? 'not published' : 'no remote') : `→ ${b.upstream}${b.gone ? ' (gone)' : ''}`);
    upstream.classList.toggle('warn', b.gone);
    upstream.title = upstream.textContent ?? '';
    where.appendChild(upstream);
  } else {
    where.append(icon('git-commit'), h('span', 'branch', s.detached ? `Detached HEAD at ${s.detached}` : 'No commits yet'));
  }
  body.appendChild(where);

  if (b || s.hasRemotes) {
    const sync = h('div', 'sync');
    if (s.hasRemotes) {
      const fetch = h('button', 'button');
      fetch.append(icon('refresh'), h('span', '', 'Fetch'));
      fetch.title = 'Download what is new on the remote, without changing your branches';
      fetch.addEventListener('click', () => run('slothgit.sync.fetch'));
      sync.appendChild(fetch);
    }
    if (b) {
      const tracking = !!b.upstream && !b.gone;
      sync.appendChild(
        splitButton('pull', `Pull${tracking && b.behind ? ` ${b.behind}` : ''}`, tracking ? `Pull from ${b.upstream}` : 'The branch has no upstream to pull from', 'slothgit.sync.pull', tracking, tracking && b.behind > 0),
      );
      sync.appendChild(
        splitButton(
          'push',
          tracking ? `Push${b.ahead ? ` ${b.ahead}` : ''}` : 'Publish Branch',
          tracking ? `Push to ${b.upstream}` : `Push ${b.name} to a remote and track it`,
          'slothgit.sync.push',
          s.hasRemotes,
          tracking ? b.ahead > 0 && b.behind === 0 : s.hasRemotes,
        ),
      );
    }
    body.appendChild(sync);
  }

  if (s.operation) {
    const banner = h('div', 'banner');
    banner.append(icon('git-merge'), h('span', 'text', `${s.operation}${s.todo ? ` · ${s.todo} ${s.todo === 1 ? 'step' : 'steps'} left` : ''}`));
    const buttons: [string, string, string][] = [
      ['slothgit.operation.continue', 'Continue', 'debug-continue'],
      ...(s.operation !== 'Merging' ? [['slothgit.operation.skip', 'Skip', 'debug-step-over'] as [string, string, string]] : []),
      ['slothgit.operation.abort', 'Abort', 'debug-stop'],
    ];
    for (const [command, label, i] of buttons) {
      const button = h('button', `button${command.endsWith('continue') ? ' primary' : ''}`);
      button.append(icon(i), h('span', '', label));
      button.addEventListener('click', () => run(command));
      banner.appendChild(button);
    }
    body.appendChild(banner);
  } else if (s.bisecting) {
    const banner = h('div', 'banner');
    banner.append(icon('bug'), h('span', 'text', 'Bisecting: mark commits good or bad from their menu'));
    const reset = h('button', 'button');
    reset.append(icon('debug-stop'), h('span', '', 'Stop Bisecting'));
    reset.addEventListener('click', () => run('slothgit.bisect.reset'));
    banner.appendChild(reset);
    body.appendChild(banner);
  }
}

// --- commit box (in the Files panel) ------------------------------------------------------------

function buildCommitBox(panel: PanelEls): void {
  const box = h('div', 'commit-box');
  const text = h('textarea');
  text.placeholder = 'Commit message';
  text.rows = 1;
  text.value = state.draft;
  let draftTimer: ReturnType<typeof setTimeout> | undefined;
  const fit = () => {
    text.style.height = 'auto';
    text.style.height = `${Math.min(text.scrollHeight + 2, 140)}px`;
  };
  const buttons = h('div', 'row-buttons');
  const commit = h('button', 'button primary');
  const label = h('span');
  commit.append(icon('check'), label);
  const amendLabel = h('label');
  const amend = h('input');
  amend.type = 'checkbox';
  amend.checked = state.amend;
  amendLabel.append(amend, h('span', '', 'Amend'));
  amendLabel.title = 'Add the staged changes to the last commit, with this message (or its own message, if this is empty)';
  const update = () => {
    label.textContent = amend.checked ? (text.value.trim() ? 'Amend Last Commit' : 'Amend, Keeping the Message') : 'Commit';
    state.draft = text.value;
    state.amend = amend.checked;
    save();
  };
  text.addEventListener('input', () => {
    fit();
    update();
    // Shared with the Source Control view's message box, which keeps it when the tab closes.
    clearTimeout(draftTimer);
    draftTimer = setTimeout(() => vscode.postMessage({ type: 'draft', message: text.value }), 300);
  });
  amend.addEventListener('change', update);
  commit.addEventListener('click', () => vscode.postMessage({ type: 'commit', message: text.value, amend: amend.checked }));
  buttons.append(commit, amendLabel);
  box.append(text, buttons);
  panel.section.appendChild(box);
  update();
  requestAnimationFrame(fit);
  window.addEventListener('message', (event: MessageEvent<ToTab>) => {
    if (event.data.type === 'draft') {
      if (document.activeElement !== text && event.data.message !== text.value) {
        text.value = event.data.message;
        fit();
        update();
      }
      return;
    }
    if (event.data.type !== 'committed') return;
    text.value = '';
    amend.checked = false;
    fit();
    update();
  });
}

// --- detail pane --------------------------------------------------------------------------------

const detailBody = $<HTMLDivElement>('detail-body');
const detailTitle = $<HTMLSpanElement>('detail-title');
const back = $<HTMLButtonElement>('back');
back.addEventListener('click', () => vscode.postMessage({ type: 'back' }));

function renderDiff(block: Extract<DetailBlock, { type: 'diff' }>): HTMLElement {
  const wrap = h('div', 'block diff');
  if (block.title) wrap.appendChild(h('h3', 'block-title', block.title));
  if (!block.files.length) wrap.appendChild(h('div', 'text-block', 'No changes.'));
  for (const file of block.files) wrap.appendChild(renderDiffFile(file));
  if (block.omitted) {
    const more = h('div', 'diff-more');
    more.appendChild(h('span', '', `${block.omitted} more ${block.omitted === 1 ? 'line' : 'lines'} not shown.`));
    const all = h('button', 'button', 'Show the Whole Diff');
    all.addEventListener('click', () => vscode.postMessage({ type: 'fullDiff' }));
    more.appendChild(all);
    wrap.appendChild(more);
  }
  return wrap;
}

function renderDiffFile(file: DisplayFile): HTMLElement {
  const el = h('div', 'diff-file');
  const header = h('div', 'file-header');
  header.appendChild(h('span', `file-status st-${file.status}`, file.status));
  const name = h('span', 'path', file.oldPath ? `${file.oldPath} → ${file.path}` : file.path);
  name.title = name.textContent ?? '';
  header.appendChild(name);
  const stats = h('span', 'stats');
  if (file.added) stats.appendChild(h('span', 'added', `+${file.added} `));
  if (file.removed) stats.appendChild(h('span', 'removed', `−${file.removed}`));
  header.append(stats, h('span', 'spacer'));
  header.appendChild(iconButton('go-to-file', 'Open in the Diff Editor', () => vscode.postMessage({ type: 'openDiff', path: file.path })));
  header.addEventListener('click', () => el.classList.toggle('collapsed'));
  el.appendChild(header);
  const hunks = h('div', 'hunks');
  if (file.binary) hunks.appendChild(h('div', 'diff-note', 'Binary file'));
  for (const note of file.notes) hunks.appendChild(h('div', 'diff-note', note));
  if (!file.binary && !file.hunks.length && !file.notes.length) hunks.appendChild(h('div', 'diff-note', file.status === 'R' ? 'Renamed without changes' : 'No changes shown'));
  for (const hunk of file.hunks) {
    hunks.appendChild(h('div', 'hunk-header', hunk.header));
    for (const line of hunk.lines) {
      const kind = line.kind === '+' ? ' add' : line.kind === '-' ? ' del' : line.kind === '\\' ? ' note' : '';
      const row = h('div', `line${kind}`);
      row.append(h('span', 'no', line.old === undefined ? '' : String(line.old)), h('span', 'no', line.new === undefined ? '' : String(line.new)));
      row.appendChild(h('span', 'text', line.kind === '\\' ? line.text : `${line.kind}${line.text}`));
      hunks.appendChild(row);
    }
  }
  el.appendChild(hunks);
  return el;
}

function renderDetail(detail: TabDetail): void {
  detailTitle.textContent = detail.title;
  detailTitle.title = detail.title;
  back.classList.toggle('hidden', !detail.back);
  $<HTMLSpanElement>('back-label').textContent = detail.back ?? 'Back';
  detailBody.textContent = '';
  detailBody.scrollTop = 0;
  if (!detail.blocks.length) {
    const empty = h('div', 'empty-detail');
    empty.append(
      h('div', '', 'Select a file, branch, commit or stash to see it here.'),
      h('div', '', 'Right-click a row for everything you can do with it.'),
    );
    detailBody.appendChild(empty);
    return;
  }
  for (const block of detail.blocks) {
    switch (block.type) {
      case 'message': {
        const el = h('div', 'block message');
        el.appendChild(h('h2', '', block.subject));
        if (block.body) el.appendChild(h('pre', '', block.body));
        detailBody.appendChild(el);
        break;
      }
      case 'refs': {
        const el = h('div', 'block');
        el.appendChild(pills(block.refs));
        detailBody.appendChild(el);
        break;
      }
      case 'meta': {
        const dl = h('dl', 'block meta');
        for (const item of block.items) {
          const dd = h('dd', '', item.value);
          if (item.title) dd.title = item.title;
          dl.append(h('dt', '', item.label), dd);
        }
        detailBody.appendChild(dl);
        break;
      }
      case 'rows': {
        const el = h('div', 'block');
        if (block.title) el.appendChild(h('h3', 'block-title', block.title));
        const list = h('div', 'list');
        if (!block.rows.length && block.empty) list.appendChild(h('div', 'empty', block.empty));
        for (const row of block.rows) list.appendChild(rowElement(row, 'detail'));
        list.addEventListener('mousedown', (e) => {
          if (e.button !== 0) return;
          const id = (e.target as HTMLElement).closest<HTMLElement>('.row')?.dataset.id;
          if (!id) return;
          for (const r of detailBody.querySelectorAll<HTMLElement>('.row')) r.classList.toggle('selected', r.dataset.id === id);
        });
        list.addEventListener('click', (e) => {
          if ((e.target as HTMLElement).closest('.row-actions')) return;
          const id = (e.target as HTMLElement).closest<HTMLElement>('.row')?.dataset.id;
          if (id) vscode.postMessage({ type: 'detailSelect', id });
        });
        el.appendChild(list);
        detailBody.appendChild(el);
        break;
      }
      case 'diff':
        detailBody.appendChild(renderDiff(block));
        break;
      case 'text':
        detailBody.appendChild(h('div', 'block text-block', block.text));
        break;
    }
  }
}

// --- command log ----------------------------------------------------------------------------------

const logBody = $<HTMLDivElement>('log-body');
const progress = $<HTMLDivElement>('progress');
$('clear-log').addEventListener('click', () => vscode.postMessage({ type: 'clearLog' }));

function renderLog(entries: LogEntry[]): void {
  progress.classList.toggle('active', entries[0]?.status === 'running');
  logBody.textContent = '';
  if (!entries.length) {
    logBody.appendChild(h('div', 'empty', 'Git commands SlothGit runs for you show up here.'));
    return;
  }
  for (const entry of entries) {
    const el = h('div', `log-entry ${entry.status}`);
    const head = h('div', 'head');
    const time = new Date(entry.time).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
    head.append(
      icon(entry.status === 'running' ? 'loading' : entry.status === 'failed' ? 'error' : 'check'),
      h('span', 'title', entry.title),
      h('span', 'time', time),
    );
    el.appendChild(head);
    for (const command of entry.commands) el.appendChild(h('div', 'cmd', command));
    logBody.appendChild(el);
  }
}

// --- sashes -----------------------------------------------------------------------------------------

function sash(el: HTMLElement, onDrag: (e: MouseEvent) => void): void {
  el.addEventListener('mousedown', (down) => {
    down.preventDefault();
    el.classList.add('dragging');
    const move = (e: MouseEvent) => onDrag(e);
    const up = () => {
      el.classList.remove('dragging');
      window.removeEventListener('mousemove', move);
      window.removeEventListener('mouseup', up);
      save();
    };
    window.addEventListener('mousemove', move);
    window.addEventListener('mouseup', up);
  });
}

function applySizes(): void {
  if (state.leftWidth) document.documentElement.style.setProperty('--left-width', `${state.leftWidth}px`);
  if (state.logHeight) document.documentElement.style.setProperty('--log-height', `${state.logHeight}px`);
}

sash($('sash-main'), (e) => {
  state.leftWidth = Math.max(260, Math.min(e.clientX - 6, window.innerWidth - 300));
  applySizes();
});
sash($('sash-log'), (e) => {
  state.logHeight = Math.max(60, Math.min(window.innerHeight - e.clientY - 6, window.innerHeight - 160));
  applySizes();
});

// --- startup and messages ---------------------------------------------------------------------------

left.appendChild(status.section);
for (const def of PANELS) {
  const panel = buildPanel(def);
  panels.set(def.id, panel);
  left.appendChild(panel.section);
  if (def.id === 'files') buildCommitBox(panel);
  updatePanel(panel);
}
applySizes();
focusPanel(state.focused);

function reportRendered(): void {
  const counts: Partial<Record<ListId, number>> = {};
  for (const [id, list] of lists) counts[id] = list.rows.length;
  vscode.postMessage({ type: 'rendered', lists: counts });
}

window.addEventListener('message', (event: MessageEvent<ToTab>) => {
  const m = event.data;
  switch (m.type) {
    case 'init':
      commands = m.commands;
      for (const p of panels.values()) updatePanel(p);
      break;
    case 'list':
      lists.set(m.list.id, m.list);
      selections.set(m.list.id, (selections.get(m.list.id) ?? []).filter((id) => m.list.rows.some((r) => r.id === id)));
      renderList(m.list.id);
      reportRendered();
      break;
    case 'status':
      renderStatus(m.status);
      break;
    case 'detail':
      renderDetail(m.detail);
      break;
    case 'log':
      renderLog(m.entries);
      break;
    case 'select': {
      select(m.list, m.ids, false);
      const panel = panelOfList.get(m.list);
      const row = panel?.bodies.get(m.list)?.querySelector<HTMLElement>(`.row[data-id="${CSS.escape(m.ids[0] ?? '')}"]`);
      row?.scrollIntoView({ block: 'nearest' });
      break;
    }
    case 'show': {
      const panel = panelOfList.get(m.list);
      if (!panel) break;
      switchTab(panel, m.list);
      focusPanel(panel.def.id);
      break;
    }
  }
});

renderDetail({ title: '', key: '', blocks: [] });
renderLog([]);
vscode.postMessage({ type: 'ready' });
