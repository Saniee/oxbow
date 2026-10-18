import * as vscode from 'vscode';
import { setContext } from './contextKeys';
import { buildEditorCommand, GitOps, type OpsOptions } from './git/operations';
import { Git } from './git/runner';
import { repoSignature } from './git/signature';
import type { API, Repository } from './types/git';

/** Tracks which repository SlothGit is showing and signals when it (or its state) changes. */
export class RepoManager implements vscode.Disposable {
  private current: Repository | undefined;
  private stateListener: vscode.Disposable | undefined;
  private debounce: NodeJS.Timeout | undefined;
  /** repoSignature when the views last loaded. */
  private signature: string | undefined;
  private readonly disposables: vscode.Disposable[] = [];
  private readonly editorCommand: string;

  private readonly changeEmitter = new vscode.EventEmitter<void>();
  /** Fires (debounced) when the selected repository changes or its state changes. */
  readonly onDidChange = this.changeEmitter.event;

  private readonly workingTreeEmitter = new vscode.EventEmitter<void>();
  /** Fires (debounced) on every state change of the repository, edits to files included. */
  readonly onDidChangeWorkingTree = this.workingTreeEmitter.event;

  constructor(
    readonly api: API,
    editorScript: string,
  ) {
    this.editorCommand = buildEditorCommand(process.execPath, editorScript);
    this.disposables.push(
      this.changeEmitter,
      this.workingTreeEmitter,
      api.onDidOpenRepository(() => this.ensureRepository()),
      api.onDidCloseRepository(() => this.ensureRepository()),
      vscode.window.onDidChangeActiveTextEditor((editor) => {
        const repo = editor && this.api.getRepository(editor.document.uri);
        if (repo) this.select(repo);
      }),
    );
    this.ensureRepository();
  }

  get repository(): Repository | undefined {
    return this.current;
  }

  git(): Git | undefined {
    return this.current && new Git(this.api.git.path, this.current.rootUri.fsPath);
  }

  ops(): GitOps | undefined {
    const git = this.git();
    const updateStackedBranches = vscode.workspace.getConfiguration('slothgit').get<OpsOptions['updateStackedBranches']>('updateStackedBranches');
    return git && new GitOps(git, this.editorCommand, { updateStackedBranches });
  }

  /** Call when the views (re)load, so later state events that change nothing they show don't reload them again. */
  async markLoaded(): Promise<void> {
    this.signature = await this.currentSignature();
  }

  private async currentSignature(): Promise<string | undefined> {
    const git = this.git();
    return git && repoSignature(git).catch(() => undefined);
  }

  async pickRepository(): Promise<void> {
    const items = this.api.repositories.map((repo) => ({
      label: vscode.workspace.asRelativePath(repo.rootUri, false) || repo.rootUri.fsPath,
      description: repo === this.current ? 'current' : undefined,
      repo,
    }));
    const picked = await vscode.window.showQuickPick(items, { placeHolder: 'Repository to show in SlothGit' });
    if (picked) this.select(picked.repo);
  }

  private ensureRepository(): void {
    setContext('slothgit.multipleRepos', this.api.repositories.length > 1);
    if (this.current && this.api.repositories.includes(this.current)) return;
    const editor = vscode.window.activeTextEditor;
    const fromEditor = editor ? this.api.getRepository(editor.document.uri) : null;
    this.select(fromEditor ?? this.api.repositories[0]);
  }

  private select(repo: Repository | undefined): void {
    if (repo === this.current) return;
    this.current = repo;
    this.signature = undefined;
    this.stateListener?.dispose();
    this.stateListener = repo?.state.onDidChange(() => this.scheduleChange());
    setContext('slothgit.hasRepository', !!repo);
    this.changeEmitter.fire();
  }

  private scheduleChange(): void {
    clearTimeout(this.debounce);
    // The Git extension reports every change, including edits to files; most of SlothGit's lists only show refs
    // and history. The Files panel listens to onDidChangeWorkingTree, which fires for every change.
    this.debounce = setTimeout(async () => {
      this.workingTreeEmitter.fire();
      const signature = await this.currentSignature();
      if (signature !== undefined && signature === this.signature) return;
      this.changeEmitter.fire();
    }, 250);
  }

  dispose(): void {
    clearTimeout(this.debounce);
    this.stateListener?.dispose();
    this.disposables.forEach((d) => d.dispose());
  }
}
