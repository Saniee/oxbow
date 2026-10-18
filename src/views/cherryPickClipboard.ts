import * as vscode from 'vscode';
import { setContext } from '../contextKeys';
import type { CommitNode } from './nodes';

/** Commits copied for cherry-picking, kept oldest first (the order they will be applied). */
export class CherryPickClipboard implements vscode.Disposable {
  private commits: CommitNode[] = [];
  private readonly emitter = new vscode.EventEmitter<void>();
  readonly onDidChange = this.emitter.event;

  get items(): readonly CommitNode[] {
    return this.commits;
  }

  has(sha: string): boolean {
    return this.commits.some((c) => c.sha === sha);
  }

  /** Adds the nodes, or removes them if all are already copied (lazygit toggles). */
  toggle(nodes: CommitNode[]): void {
    if (nodes.every((n) => this.has(n.sha))) {
      this.commits = this.commits.filter((c) => !nodes.some((n) => n.sha === c.sha));
    } else {
      const added = nodes.filter((n) => !this.has(n.sha)).sort((a, b) => b.index - a.index);
      this.commits = [...this.commits, ...added];
    }
    this.changed();
  }

  clear(): void {
    this.commits = [];
    this.changed();
  }

  private changed(): void {
    setContext('slothgit.hasCherryPicks', this.commits.length > 0);
    this.emitter.fire();
  }

  dispose(): void {
    this.emitter.dispose();
  }
}
