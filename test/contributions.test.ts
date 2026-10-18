// package.json's contributions are written by hand. These checks catch what VS Code itself would silently ignore:
// menu entries for commands that don't exist, commands nothing registers, and menus that drift apart.
import assert from 'node:assert/strict';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test } from 'node:test';
import { BUTTON_COMMANDS, LIST_ACTIONS, ROW_ACTIONS, STATUS_ACTIONS } from '../src/tab/actions';
import { evaluate } from '../src/tab/when';
import { THEME_COLOR_DEFAULTS } from '../src/themeColors';
import { SYNC_MENUS } from '../src/webview/syncMenus';

interface MenuEntry {
  command?: string;
  submenu?: string;
  when?: string;
  group?: string;
}

const root = path.join(__dirname, '..');
const pkg = JSON.parse(fs.readFileSync(path.join(root, 'package.json'), 'utf8'));
const contributes = pkg.contributes;
const commands = new Set<string>(contributes.commands.map((c: { command: string }) => c.command));
const submenus = new Set<string>(contributes.submenus.map((s: { id: string }) => s.id));
const menus: Record<string, MenuEntry[]> = contributes.menus;
const TAB = "webviewId == 'slothgit.tab'";

/** All TypeScript under src, as one string to search for ids. */
function sources(dir = path.join(root, 'src')): string {
  return fs
    .readdirSync(dir, { withFileTypes: true })
    .map((e) => (e.isDirectory() ? sources(path.join(dir, e.name)) : e.name.endsWith('.ts') ? fs.readFileSync(path.join(dir, e.name), 'utf8') : ''))
    .join('\n');
}
const src = sources();

test('every menu entry points at a contributed command or submenu', () => {
  for (const [menu, entries] of Object.entries(menus)) {
    for (const e of entries) {
      if (e.command) assert.ok(commands.has(e.command), `${menu}: unknown command ${e.command}`);
      if (e.submenu) assert.ok(submenus.has(e.submenu), `${menu}: unknown submenu ${e.submenu}`);
    }
  }
  for (const id of submenus) assert.ok(menus[id]?.length, `submenu ${id} has no entries`);
});

test('every contributed command is registered in the code, and every registered one is contributed', () => {
  for (const id of commands) assert.ok(src.includes(`'${id}'`), `${id} is contributed but never registered`);
  // Registrations are object keys ('slothgit.x': handler) or registerCommand('slothgit.x', …).
  const registered = [...src.matchAll(/'(slothgit\.[\w.]+)'\s*(?::|,\s*\()/g)].map((m) => m[1]);
  for (const id of registered) {
    if (/^slothgit\.(tab|graph|launcher)$/.test(id)) continue; // webview and view ids
    if (contributes.colors.some((c: { id: string }) => c.id === id)) continue; // theme colours in a ternary
    assert.ok(commands.has(id), `${id} is registered but not in package.json contributes.commands`);
  }
});

test('every view is created in the code', () => {
  for (const views of Object.values<{ id: string }[]>(contributes.views)) {
    for (const view of views) assert.ok(src.includes(`'${view.id}'`), `view ${view.id} is never created`);
  }
});

test('no menu refers to the Source Control views SlothGit no longer has', () => {
  for (const [menu, entries] of Object.entries(menus)) {
    for (const e of entries) {
      assert.doesNotMatch(e.when ?? '', /\bview(Item)? *(==|=~)|slothgit\.(sync|commits|branches|stashes|reflog)'/, `${menu}: ${e.command ?? e.submenu} still uses a tree view's when clause`);
    }
  }
  const walkthrough = JSON.stringify(contributes.walkthroughs);
  assert.doesNotMatch(walkthrough, /\.focus\)/, 'the walkthrough opens views that no longer exist');
});

test('the tab\'s buttons run contributed commands that have an icon', () => {
  for (const id of BUTTON_COMMANDS) assert.ok(commands.has(id), `a button runs ${id}, which is not contributed`);
  const icons = new Map<string, string | undefined>(contributes.commands.map((c: { command: string; icon?: string }) => [c.command, c.icon]));
  const primary = [...ROW_ACTIONS, ...Object.values(LIST_ACTIONS).flat().filter((a) => a.primary), ...STATUS_ACTIONS.filter((a) => a.primary)];
  for (const a of primary) assert.match(icons.get(a.command) ?? '', /^\$\([\w-]+\)$/, `${a.command} is drawn as an icon but has none`);
});

test('row buttons also appear in the row\'s right-click menu', () => {
  // A button on a row is a shortcut; everything it does must be findable in the menu too.
  const tabMenu = menus['webview/context'].filter((e) => e.when?.includes(TAB));
  const samples: Record<string, Record<string, string>> = {
    'slothgit.commit.openChanges': { slothgitView: 'commits', slothgitItem: 'commit' },
    'slothgit.commit.moveUp': { slothgitView: 'commits', slothgitItem: 'commit' },
    'slothgit.commit.moveDown': { slothgitView: 'commits', slothgitItem: 'commit:head' },
    'slothgit.todo.moveUp': { slothgitView: 'commits', slothgitItem: 'todo:pick' },
    'slothgit.todo.moveDown': { slothgitView: 'commits', slothgitItem: 'todo:pick' },
    'slothgit.stash.apply': { slothgitView: 'stashes', slothgitItem: 'stash' },
    'slothgit.stash.pop': { slothgitView: 'stashes', slothgitItem: 'stash' },
    'slothgit.branch.checkout': { slothgitView: 'branches', slothgitItem: 'branch:local' },
    'slothgit.branch.fastForward': { slothgitView: 'branches', slothgitItem: 'branch:local:upstream' },
    'slothgit.branch.push': { slothgitView: 'branches', slothgitItem: 'branch:local' },
    'slothgit.remote.fetch': { slothgitView: 'branches', slothgitItem: 'remote' },
    'slothgit.worktree.open': { slothgitView: 'branches', slothgitItem: 'worktree' },
    'slothgit.file.openAtRevision': { slothgitView: 'commits', slothgitItem: 'file:log' },
    'slothgit.file.openMergeEditor': { slothgitView: 'files', slothgitItem: 'change:conflict' },
    'slothgit.file.open': { slothgitView: 'files', slothgitItem: 'change:unstaged' },
    'slothgit.file.discard': { slothgitView: 'files', slothgitItem: 'change:unstaged' },
    'slothgit.file.stage': { slothgitView: 'files', slothgitItem: 'change:unstaged' },
    'slothgit.file.unstage': { slothgitView: 'files', slothgitItem: 'change:staged' },
  };
  for (const action of ROW_ACTIONS) {
    const sample = samples[action.command];
    assert.ok(sample, `add a sample row for ${action.command}`);
    const values = { webviewId: 'slothgit.tab', ...sample, view: sample.slothgitView, item: sample.slothgitItem };
    assert.ok(evaluate(action.when, values), `the sample row for ${action.command} gets the button`);
    const inMenu = tabMenu.some((e) => e.command === action.command && evaluate(e.when, values));
    assert.ok(inMenu, `${action.command} is a button on ${sample.slothgitItem} rows but not in their right-click menu`);
  }
});

test("the Status panel's right-click menus match its ⌄ quick picks", () => {
  for (const [menu, items] of Object.entries(SYNC_MENUS)) {
    const entries = menus['webview/context'].filter((e) => e.when?.includes(`webviewSection == '${menu}'`) && e.when.includes(TAB));
    assert.deepEqual(
      entries.map((e) => e.command),
      items.map((i) => i.command),
      `${menu} menu: same commands in the same order`,
    );
    for (const [i, item] of items.entries()) {
      assert.equal(entries[i].when!.includes('!config.slothgit.simpleMode'), !!item.advanced, `${item.command}: hidden in Simple Mode`);
    }
  }
});

test("the Commit Graph's context menu only offers commands the Commits panel has", () => {
  // Entries for commit rows of the tab's Commits list: slothgitItem == commit, a pattern naming commit (not only
  // refcommit), or anything in the Commits list not limited to some kind of row.
  const commitMenu = (when = '') =>
    when.includes(TAB) &&
    (/slothgitItem (=~ \/[^/]*\bcommit\b|== commit\b)/.test(when) || (when.includes("slothgitView == 'commits'") && !when.includes('slothgitItem')));
  const inTab = new Set<string>();
  for (const e of menus['webview/context']) {
    if (!commitMenu(e.when)) continue;
    if (e.command) inTab.add(e.command);
    if (e.submenu) for (const s of menus[e.submenu]) if (s.command) inTab.add(s.command);
  }
  for (const e of menus['webview/context'].filter((x) => x.when?.includes("'slothgit.graph'"))) {
    const ids = e.command ? [e.command] : (menus[e.submenu!] ?? []).map((s) => s.command!);
    for (const id of ids) assert.ok(inTab.has(id), `the graph offers ${id}, which the Commits panel's commit menu doesn't`);
  }
});

test('Simple Mode hides the same actions everywhere they appear', () => {
  // An action hidden in Simple Mode in one menu should not show up ungated in another (the Command Palette aside).
  const isGated = (e: MenuEntry) => !!e.when?.includes('!config.slothgit.simpleMode');
  // A submenu hidden wherever it is referenced hides everything in it.
  const hiddenSubmenus = new Set(
    [...submenus].filter((id) =>
      Object.values(menus)
        .flat()
        .filter((e) => e.submenu === id)
        .every(isGated),
    ),
  );
  const gated = new Set<string>();
  const ungated = new Map<string, string>();
  const tabButtons: [string, MenuEntry[]][] = [
    ['row buttons', ROW_ACTIONS],
    ['list headers', Object.values(LIST_ACTIONS).flat()],
    ['Status panel', STATUS_ACTIONS],
  ];
  for (const [menu, entries] of [...Object.entries(menus), ...tabButtons]) {
    if (menu === 'commandPalette') continue;
    for (const e of entries) {
      const id = e.command ?? e.submenu!;
      if (isGated(e) || hiddenSubmenus.has(menu)) gated.add(id);
      else ungated.set(id, menu);
    }
  }
  // Ways out: leaving Simple Mode, and stopping a bisect that was started before it was turned on.
  const allowed = new Set(['slothgit.simpleMode.off', 'slothgit.bisect.reset']);
  for (const id of gated) {
    if (ungated.has(id) && !allowed.has(id)) assert.fail(`${id} is hidden in Simple Mode but still shown in ${ungated.get(id)}`);
  }
});

test('every slothgit.* colour the code or a page uses is contributed, so themes can set it', () => {
  const contributed = new Set<string>(contributes.colors.map((c: { id: string }) => c.id));
  const css = ['tab', 'graph'].map((page) => fs.readFileSync(path.join(root, 'src', 'webview', page, 'style.css'), 'utf8')).join('\n');
  const used = new Set([
    ...[...src.matchAll(/'(slothgit\.\w+(?:Icon|Foreground|Background))'/g)].map((m) => m[1]),
    ...[...css.matchAll(/--vscode-slothgit-(\w+)/g)].map((m) => `slothgit.${m[1]}`),
  ]);
  assert.ok(used.size > 10, 'found the colours in use');
  for (const id of used) assert.ok(contributed.has(id), `${id} is in contributes.colors`);
  for (const id of contributed) assert.ok(used.has(id), `${id} is used somewhere`);
});

test('pages fall back to each slothgit.* colour\'s default, for when VS Code has not passed the page its colours', () => {
  const defaults = Object.fromEntries(contributes.colors.map((c: { id: string; defaults: { dark: string } }) => [c.id, c.defaults.dark]));
  assert.deepEqual(THEME_COLOR_DEFAULTS, defaults, 'src/themeColors.ts matches package.json');
  for (const page of ['tab', 'graph']) {
    const css = fs.readFileSync(path.join(root, 'src', 'webview', page, 'style.css'), 'utf8');
    for (const m of css.matchAll(/var\(--vscode-slothgit-(\w+)(?:, var\(--vscode-([\w-]+)\))?\)/g)) {
      const id = `slothgit.${m[1]}`;
      assert.equal(m[2], defaults[id]?.replace(/\./g, '-'), `${page}/style.css: ${id} falls back to ${defaults[id]}`);
    }
  }
});
