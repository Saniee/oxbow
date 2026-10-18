// "Pretty mode": the commit history as a graph in an editor panel, for when the Commits view's plain list
// isn't enough to see how branches relate. Data comes from the same queries; right-click actions are the
// Commits view's own commands, through VS Code's webview context menu.
import * as vscode from 'vscode';
import { layoutGraph, parseRefs, type GraphRow } from '../git/graph';
import { currentBranch, getCommitMessage, getLog, getMainRefs, getRemotes, getUnpushed, notOnHead, reachableFrom, type Commit } from '../git/queries';
import type { RepoManager } from '../repoManager';
import type { FromGraph, GraphCommit, GraphState, ToGraph } from '../webview/messages';
import { pageHtml, webviewOptions } from './webviewShell';
import body from '../webview/graph/body.html';
import type { CommitNode, FileNode } from './nodes';
import { commitFiles, config } from './providers';
import { registerWebviewNodes } from './webviewNodes';

export class GraphPanel implements vscode.Disposable {
  private static current: GraphPanel | undefined;

  /** Commits currently shown, by SHA, for commands run from the panel's context menu. */
  private nodes = new Map<string, CommitNode>();
  private search = '';
  private all = true;
  private readonly disposables: vscode.Disposable[] = [];

  static show(context: vscode.ExtensionContext, repos: RepoManager): void {
    if (GraphPanel.current) {
      GraphPanel.current.panel.reveal();
      return;
    }
    GraphPanel.current = new GraphPanel(context, repos);
  }

  /** The commit a graph context-menu command was run on. */
  static commit(sha: string): CommitNode | undefined {
    return GraphPanel.current?.nodes.get(sha);
  }

  /** How many rows the open page last drew, for tests. */
  static renderedRows(): number | undefined {
    return GraphPanel.current?.rendered;
  }

  private rendered: number | undefined;

  private readonly panel: vscode.WebviewPanel;

  private constructor(
    context: vscode.ExtensionContext,
    private readonly repos: RepoManager,
  ) {
    this.panel = vscode.window.createWebviewPanel('slothgit.graph', 'Commit Graph', vscode.ViewColumn.Active, {
      ...webviewOptions(context.extensionUri),
      retainContextWhenHidden: true,
    });
    this.panel.webview.html = pageHtml(this.panel.webview, context.extensionUri, 'graph', 'Commit Graph', body);
    this.disposables.push(
      this.panel.onDidDispose(() => this.dispose()),
      this.panel.webview.onDidReceiveMessage((m: FromGraph) => this.onMessage(m)),
      this.panel.onDidChangeViewState((e) => e.webviewPanel.visible && this.load()),
      repos.onDidChange(() => this.panel.visible && this.load()),
    );
    context.subscriptions.push(this);
  }

  private post(message: ToGraph): void {
    void this.panel.webview.postMessage(message);
  }

  private async onMessage(message: FromGraph): Promise<void> {
    switch (message.type) {
      case 'rendered':
        this.rendered = message.rows;
        return;
      case 'ready':
      case 'refresh':
        return this.load();
      case 'search':
        this.search = message.query.trim();
        return this.load();
      case 'allBranches':
        this.all = message.value;
        return this.load();
      case 'select':
        return this.details(message.sha);
      case 'open': {
        const node = this.nodes.get(message.sha);
        if (node) await vscode.commands.executeCommand('slothgit.commit.openChanges', node);
        return;
      }
      case 'openFile': {
        const node = this.nodes.get(message.sha);
        const git = this.repos.git();
        if (!node || !git) return;
        const file = (await commitFiles(git, node)).find((f: FileNode) => f.pair.change.path === message.path);
        if (file) await vscode.commands.executeCommand('slothgit.openFileDiff', file);
        return;
      }
    }
  }

  private async load(): Promise<void> {
    const git = this.repos.git();
    if (!git) {
      this.post({ type: 'rows', rows: [], state: this.state(undefined) });
      return;
    }
    const limit = config('commitLimit', 300);
    const [log, remotes, unpushed, head, branch, mainRefs] = await Promise.all([
      getLog(git, limit, 'HEAD', [], { all: this.all, search: this.search }),
      getRemotes(git),
      getUnpushed(git),
      git.exec(['rev-parse', '-q', '--verify', 'HEAD'], { allowedExitCodes: [1, 128] }),
      currentBranch(git),
      getMainRefs(git, config<string[]>('mainBranches', ['main', 'master'])),
    ]);
    const shas = log.map((c) => c.sha);
    const [others, merged] = await Promise.all([notOnHead(git, shas), reachableFrom(git, shas, mainRefs)]);
    const headSha = head.stdout.trim();
    const remoteNames = remotes.map((r) => r.name);
    // Lanes only make sense for an unbroken history; a search result is drawn as single dots.
    const graph = this.search ? log.map((): GraphRow => ({ lane: 0, color: 0, lines: [], width: 1 })) : layoutGraph(log);

    this.nodes = new Map(
      log.map((c, index): [string, CommitNode] => [c.sha, { kind: 'commit', source: 'log', index, ...c, onHead: !others.has(c.sha) }]),
    );
    const rows = log.map((c: Commit, i): GraphCommit => ({
      sha: c.sha,
      shortSha: c.shortSha,
      subject: c.subject,
      author: c.author,
      email: c.authorEmail,
      coAuthors: c.coAuthors,
      relDate: c.relDate,
      date: c.date,
      parents: c.parents,
      refs: parseRefs(c.refs, remoteNames),
      onHead: !others.has(c.sha),
      pushed: unpushed === null || others.has(c.sha) ? null : !unpushed.has(c.sha),
      merged: merged.has(c.sha),
      head: c.sha === headSha,
      graph: graph[i],
    }));
    this.panel.title = `Commit Graph${branch ? `: ${branch}` : ''}`;
    this.post({ type: 'rows', rows, state: this.state(rows.length === limit ? limit : undefined) });
  }

  private state(limit: number | undefined): GraphState {
    return { search: this.search, all: this.all, limit };
  }

  private async details(sha: string): Promise<void> {
    const node = this.nodes.get(sha);
    const git = this.repos.git();
    if (!node || !git) return;
    const [message, files] = await Promise.all([getCommitMessage(git, sha), commitFiles(git, node)]);
    this.post({
      type: 'details',
      sha,
      message,
      files: files.map((f) => ({ status: f.pair.change.status, path: f.pair.change.path, oldPath: f.pair.change.oldPath })),
    });
  }

  dispose(): void {
    if (GraphPanel.current === this) GraphPanel.current = undefined;
    this.disposables.forEach((d) => d.dispose());
    this.disposables.length = 0;
    this.panel.dispose();
  }
}

/** Commands run from the graph's context menu get the commit node of the row ({ sha } in its webview context). */
export function registerGraphNodes(): vscode.Disposable {
  return registerWebviewNodes('slothgit.graph', (context) => {
    const node = typeof context.sha === 'string' ? GraphPanel.commit(context.sha) : undefined;
    return node ? [node] : [];
  });
}
