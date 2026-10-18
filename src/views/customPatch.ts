import * as vscode from 'vscode';
import { setContext } from '../contextKeys';
import { addToSelection, WHOLE_FILE, type LineRange, type Selection, type Side } from '../git/patch';
import type { CommitNode, FileNode } from './nodes';

export interface PatchEntry {
  file: FileNode;
  selection: Selection;
}

/**
 * lazygit's custom patch: changes picked from one commit, across files and selections, collected until you act
 * on them (remove them from the commit, move them to another commit or the index, apply them to the working tree).
 */
export class CustomPatch implements vscode.Disposable {
  private source: CommitNode | undefined;
  private readonly files = new Map<string, PatchEntry>();
  private readonly status = vscode.window.createStatusBarItem(vscode.StatusBarAlignment.Left, 99);
  private readonly emitter = new vscode.EventEmitter<void>();
  readonly onDidChange = this.emitter.event;

  constructor() {
    this.status.command = 'slothgit.patch.menu';
    this.status.tooltip = 'SlothGit: custom patch. Click for what to do with it.';
  }

  /** The commit the changes come from. */
  get commit(): CommitNode | undefined {
    return this.source;
  }

  get entries(): PatchEntry[] {
    return [...this.files.values()];
  }

  /** Adds lines picked on one side of a file's diff; a whole file when `ranges` is omitted. */
  add(file: FileNode, side?: Side, ranges?: LineRange[]): void {
    const commit = file.commit!;
    if (this.source && this.source.sha !== commit.sha) this.files.clear();
    this.source = commit;
    const path = file.pair.change.path;
    const current = this.files.get(path)?.selection ?? {};
    const selection = side && ranges ? addToSelection(current, side, ranges) : WHOLE_FILE;
    this.files.set(path, { file, selection });
    this.changed();
  }

  clear(): void {
    this.source = undefined;
    this.files.clear();
    this.changed();
  }

  private changed(): void {
    const count = this.files.size;
    setContext('slothgit.hasPatch', count > 0);
    if (count && this.source) {
      this.status.text = `$(diff) Patch: ${count} file${count === 1 ? '' : 's'} from ${this.source.shortSha}`;
      this.status.show();
    } else {
      this.status.hide();
    }
    this.emitter.fire();
  }

  dispose(): void {
    this.status.dispose();
    this.emitter.dispose();
  }
}
