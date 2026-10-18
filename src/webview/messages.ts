// Messages between the extension and its webview pages (src/webview/<page>/main.ts). Both sides import these
// types, so a change on one side that the other doesn't handle is a type error, not a silent no-op.
import type { LogEntry } from '../git/commandLog';
import type { DisplayFile } from '../git/diff';
import type { GraphRef, GraphRow } from '../git/graph';
import type { Person } from '../git/queries';
import type { ListId } from '../tab/actions';

// --- Commit Graph -------------------------------------------------------------------------------

/** One row as the graph page receives it. */
export interface GraphCommit {
  sha: string;
  shortSha: string;
  subject: string;
  author: string;
  email: string;
  coAuthors: Person[];
  relDate: string;
  date: string;
  parents: string[];
  refs: GraphRef[];
  onHead: boolean;
  /** null when the branch has no upstream. */
  pushed: boolean | null;
  /** On a main branch (slothgit.mainBranches). */
  merged: boolean;
  head: boolean;
  graph: GraphRow;
}

export interface GraphState {
  search: string;
  all: boolean;
  /** Set when the log was cut off at slothgit.commitLimit. */
  limit?: number;
}

export interface GraphFile {
  status: string;
  path: string;
  oldPath?: string;
}

export type ToGraph =
  | { type: 'rows'; rows: GraphCommit[]; state: GraphState }
  | { type: 'details'; sha: string; message: string; files: GraphFile[] };

export type FromGraph =
  | { type: 'ready' | 'refresh' }
  | { type: 'search'; query: string }
  | { type: 'allBranches'; value: boolean }
  | { type: 'select'; sha: string }
  | { type: 'open'; sha: string }
  | { type: 'openFile'; sha: string; path: string }
  /** After each render, so tests can tell the page really drew the rows. */
  | { type: 'rendered'; rows: number };

// --- SlothGit tab -------------------------------------------------------------------------------

export type { ListId };

/** A codicon, with a colour as a CSS value (e.g. var(--vscode-charts-green)). */
export interface TabIcon {
  id: string;
  color?: string;
}

/** What a commit row shows, laid out by the page. */
export interface TabCommit {
  shortSha: string;
  /** Author's and co-authors' initials, "AL+GH". */
  people: string;
  subject: string;
  refs: GraphRef[];
  date: string;
}

/** One row of a list, or of a list in the detail pane. */
export interface TabRow {
  /** Unique within the page; the extension maps it back to its node. */
  id: string;
  label: string;
  /** Ranges of the label to emphasise. */
  highlights?: [number, number][];
  description?: string;
  tooltip?: string;
  icon?: TabIcon;
  /** The kind of list it comes from, for menus: files, branches, commits, reflog, stashes. */
  view: string;
  /** Its kind and state, for menus (what tree items had as contextValue). */
  item: string;
  /** Commands of the buttons shown while the mouse is over it. */
  actions: string[];
  /** Rows under another row (a remote's branches) name it; they are hidden while it is collapsed. */
  parent?: string;
  /** Rows other rows name as parent. */
  expandable?: boolean;
  commit?: TabCommit;
  /** git's two status letters, for files in the Files panel; one letter for files of a commit. */
  status?: string;
  /** Not on the current branch, or otherwise less important. */
  dim?: boolean;
}

/** A header button. */
export interface TabAction {
  command: string;
  /** Shown as an icon; the others are in the ⋯ menu. */
  primary: boolean;
}

export interface TabList {
  id: ListId;
  rows: TabRow[];
  /** Shown next to the title, e.g. an active search. */
  description?: string;
  actions: TabAction[];
  /** Shown when there are no rows. */
  empty: string;
}

/** Title and codicon of every command a button may run; sent once. */
export type TabCommands = Record<string, { title: string; icon?: string }>;

export interface TabStatus {
  /** Repository folder name; undefined when there is none. */
  repo?: string;
  branch?: { name: string; upstream?: string; ahead: number; behind: number; gone: boolean };
  /** Short SHA of a detached HEAD. */
  detached?: string;
  hasRemotes: boolean;
  /** "Rebasing", "Merging"…, while one is stopped. */
  operation?: string;
  /** Rebase steps left. */
  todo?: number;
  bisecting: boolean;
  actions: TabAction[];
  /** Simple Mode hides advanced actions, here the Pull and Push menus' advanced entries. */
  simple: boolean;
}

export type DetailBlock =
  | { type: 'message'; subject: string; body: string }
  /** Lines like "Author", "Date", "SHA" with their values. */
  | { type: 'meta'; items: { label: string; value: string; title?: string }[] }
  | { type: 'refs'; refs: GraphRef[] }
  | { type: 'rows'; title?: string; rows: TabRow[]; empty?: string }
  | { type: 'diff'; title?: string; files: DisplayFile[]; omitted: number; full?: boolean }
  | { type: 'text'; text: string };

export interface TabDetail {
  title: string;
  /** Identifies what is shown, so a late answer for something else is dropped. */
  key: string;
  blocks: DetailBlock[];
  /** Whether there is something to go back to (a branch's log after opening one of its commits). */
  back?: string;
}

export type ToTab =
  | { type: 'init'; commands: TabCommands }
  | { type: 'list'; list: TabList }
  | { type: 'status'; status: TabStatus }
  | { type: 'detail'; detail: TabDetail }
  | { type: 'log'; entries: LogEntry[] }
  /** Select rows (a command kept the cursor on the commit it rewrote). */
  | { type: 'select'; list: ListId; ids: string[] }
  /** Bring a panel and list to the front. */
  | { type: 'show'; list: ListId }
  /** The commit box's commit went through: clear it. */
  | { type: 'committed' }
  /** The commit message draft, shared with the Source Control view's message box. */
  | { type: 'draft'; message: string };

export type FromTab =
  | { type: 'ready' }
  /** The selection in a list changed; the first id is the one to show details for. */
  | { type: 'select'; list: ListId; ids: string[] }
  /** A row in the detail pane was clicked. */
  | { type: 'detailSelect'; id: string }
  /** Double click: the row's main action (open a commit's changes, a file's diff, check out a branch…). */
  | { type: 'open'; id: string }
  | { type: 'back' }
  /** Run a button's command, on a row or on the list's selection. */
  | { type: 'command'; command: string; id?: string; list?: ListId }
  | { type: 'commit'; message: string; amend: boolean }
  /** The commit box's text changed. */
  | { type: 'draft'; message: string }
  /** Open a file of the diff in the detail pane in VS Code's diff editor. */
  | { type: 'openDiff'; path: string }
  /** Show the whole diff, not only its start. */
  | { type: 'fullDiff' }
  | { type: 'pullPush'; menu: 'pull' | 'push' }
  | { type: 'clearLog' }
  /** After each render, so tests can tell the page really drew it. */
  | { type: 'rendered'; lists: Partial<Record<ListId, number>> };
