// lazygit's command log: what each action ran, newest first. Read-only git commands (the ones that only look at
// the repository) are left out, as lazygit does, so the log shows what changed things. No vscode dependency.
import { onGitCommand } from './runner';

export interface LogEntry {
  id: number;
  /** The action, e.g. "Squashing…" without the dots. */
  title: string;
  /** Epoch milliseconds when it started. */
  time: number;
  /** The git commands it ran, as shell-like text. */
  commands: string[];
  status: 'running' | 'done' | 'failed';
}

/** Subcommands that never change the repository. `stash list`/`show`, `config --get` and the like are checked below. */
const READ_ONLY = new Set([
  'blame', 'cat-file', 'check-ignore', 'describe', 'diff', 'diff-tree', 'for-each-ref', 'grep', 'log', 'ls-files',
  'ls-remote', 'ls-tree', 'merge-base', 'name-rev', 'rev-list', 'rev-parse', 'shortlog', 'show', 'show-ref', 'status',
  'symbolic-ref', 'var', 'version', 'worktree list', 'remote get-url', 'stash list', 'stash show', 'branch --list',
  'reflog', 'bisect log', 'bisect visualize',
]);

/** The subcommand, skipping `-c key=value` and other global options. */
function subcommand(args: readonly string[]): string[] {
  let i = 0;
  while (i < args.length && args[i].startsWith('-')) i += args[i] === '-c' || args[i] === '-C' ? 2 : 1;
  return args.slice(i);
}

export function isReadOnly(args: readonly string[]): boolean {
  const [command, next] = subcommand(args);
  if (!command) return true;
  if (READ_ONLY.has(command) || READ_ONLY.has(`${command} ${next}`)) return true;
  if (command === 'config') return args.includes('--get') || args.includes('--get-all') || args.includes('--list') || args.includes('-l');
  // Listing forms; `git branch <name>`, `git tag <name>`, `git remote add` and the like change things.
  const rest = subcommand(args).slice(1);
  if (command === 'branch') return rest.some((a) => ['--list', '--show-current', '--merged', '--no-merged', '--contains'].includes(a));
  if (command === 'tag') return rest.length === 0 || rest.includes('-l') || rest.includes('--list');
  if (command === 'remote') return rest.length === 0 || next === '-v' || next === 'show';
  return false;
}

/** As a shell would show it: arguments with spaces or quotes are quoted. */
export function formatCommand(args: readonly string[]): string {
  const quote = (a: string) => (/^[\w@%+=:,./^~{}-]+$/.test(a) ? a : `'${a.replace(/'/g, `'\\''`)}'`);
  return ['git', ...args].map(quote).join(' ');
}

const MAX_ENTRIES = 100;

export class CommandLog {
  private readonly items: LogEntry[] = [];
  private nextId = 1;
  private readonly listeners = new Set<() => void>();
  /** The running action, which collects the repository's mutating commands. */
  private current: LogEntry | undefined;
  private readonly unsubscribe: () => void;

  constructor(private readonly cwd: () => string | undefined) {
    this.unsubscribe = onGitCommand((event) => {
      if (isReadOnly(event.args) || event.cwd !== this.cwd()) return;
      const entry = this.current ?? this.push(`git ${subcommand(event.args)[0] ?? ''}`.trim(), 'done');
      entry.commands.push(formatCommand(event.args) + (event.exitCode ? `  (exit ${event.exitCode})` : ''));
      this.changed();
    });
  }

  get entries(): readonly LogEntry[] {
    return this.items;
  }

  onDidChange(listener: () => void): () => void {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  }

  /** Starts an action; git commands run until finish() are listed under it. */
  start(title: string): LogEntry {
    this.current = this.push(title.replace(/…$/, ''), 'running');
    this.changed();
    return this.current;
  }

  finish(entry: LogEntry, ok: boolean): void {
    entry.status = ok ? 'done' : 'failed';
    if (this.current === entry) this.current = undefined;
    this.changed();
  }

  clear(): void {
    this.items.length = 0;
    this.changed();
  }

  dispose(): void {
    this.unsubscribe();
    this.listeners.clear();
  }

  private push(title: string, status: LogEntry['status']): LogEntry {
    const entry: LogEntry = { id: this.nextId++, title, time: Date.now(), commands: [], status };
    this.items.unshift(entry);
    if (this.items.length > MAX_ENTRIES) this.items.length = MAX_ENTRIES;
    return entry;
  }

  private changed(): void {
    for (const listener of this.listeners) listener();
  }
}
