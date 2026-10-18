import * as vscode from 'vscode';
import type { Git } from '../git/runner';
import type { RepoManager } from '../repoManager';
import { fileTreeItem, type Node } from './nodes';

/** What shows a provider's items (the SlothGit tab). Providers only load while it is visible. */
export interface ProviderHost {
  readonly visible: boolean;
  readonly onDidChangeVisibility: vscode.Event<boolean>;
}

/**
 * Loads root items only while its host is visible, and only notifies listeners when the data actually changed,
 * so frequent repository state events don't cause flicker or lost selection. Items are vscode.TreeItems: the
 * tab draws them as rows, with the contextValue picking their menus.
 */
export abstract class BaseProvider<TRoot extends Node> implements vscode.TreeDataProvider<Node> {
  protected roots: TRoot[] = [];
  private key = '';
  private stale = true;
  private host: ProviderHost | undefined;
  private queue: Promise<void> = Promise.resolve();
  /** The queued reload that hasn't started yet; later requests join it, as it will see their changes. */
  private pending: { force: boolean; done: Promise<void> } | undefined;

  private readonly emitter = new vscode.EventEmitter<Node | undefined | void>();
  readonly onDidChangeTreeData = this.emitter.event;

  constructor(
    protected readonly repos: RepoManager,
    private readonly output: vscode.LogOutputChannel,
  ) {}

  attach(host: ProviderHost): vscode.Disposable {
    this.host = host;
    return vscode.Disposable.from(
      this.emitter,
      host.onDidChangeVisibility((visible) => {
        if (visible && this.stale) void this.reload(true);
      }),
    );
  }

  /** The roots from the last load. */
  get items(): readonly TRoot[] {
    return this.roots;
  }

  /** Text shown next to the list's title, e.g. an active filter. */
  get summary(): string | undefined {
    return this.description();
  }

  reload(force = false): Promise<void> {
    if (this.pending) {
      this.pending.force ||= force;
      return this.pending.done;
    }
    const pending = { force, done: Promise.resolve() };
    pending.done = this.queue
      .then(() => {
        this.pending = undefined;
        return this.doReload(pending.force);
      })
      .catch((error) => this.output.error(`Failed to load ${this.constructor.name}: ${error}`));
    this.pending = pending;
    this.queue = pending.done;
    return pending.done;
  }

  /** Needed for TreeView-style reveal. Only root items are revealed, so children report no parent. */
  getParent(): Node | undefined {
    return undefined;
  }

  /** Re-render existing items without reloading data. */
  redraw(): void {
    this.emitter.fire();
  }

  private async doReload(force: boolean): Promise<void> {
    if (this.host && !this.host.visible) {
      this.stale = true;
      return;
    }
    this.stale = false;
    const git = this.repos.git();
    const roots = git ? await this.load(git) : [];
    const key = `${git?.cwd}\n${this.keyOf(roots)}`;
    if (!force && key === this.key) return;
    this.roots = roots;
    this.key = key;
    this.emitter.fire();
  }

  /** Text shown next to the list's title, e.g. an active filter. */
  protected description(): string | undefined {
    return undefined;
  }

  protected abstract load(git: Git): Promise<TRoot[]>;
  protected abstract keyOf(roots: TRoot[]): string;
  /** Tree item for any non-file node this provider produces. */
  protected abstract item(node: Node): vscode.TreeItem;
  /** Children of any non-file node this provider produces. */
  protected abstract children(git: Git, node: Node): Promise<Node[]>;

  getTreeItem(node: Node): vscode.TreeItem {
    if (node.kind === 'file') return fileTreeItem(node, this.repos.repository!.rootUri);
    return this.item(node);
  }

  async getChildren(node?: Node): Promise<Node[]> {
    if (!node) return this.roots;
    const git = this.repos.git();
    if (!git || node.kind === 'file') return [];
    return this.children(git, node);
  }
}
