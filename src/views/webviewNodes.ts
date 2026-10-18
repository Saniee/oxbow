import * as vscode from 'vscode';
import type { Node } from './nodes';

/**
 * A list of nodes on screen that commands act on when run without arguments, like a TreeView: one of the SlothGit
 * tab's lists. Commands use the selection, and keep it on the commit a rewrite produced with reveal().
 */
export interface ListView {
  readonly selection: readonly Node[];
  readonly visible: boolean;
  reveal(node: Node, options?: { select?: boolean; focus?: boolean }): Thenable<void>;
}

/** VS Code's webview context-menu argument: the element's data-vscode-context, plus the webview's id. */
export interface WebviewContext {
  webview: string;
  [key: string]: unknown;
}

type Resolver = (context: WebviewContext) => unknown[] | undefined;

const resolvers = new Map<string, Resolver>();

/** Lets commands run from `webviewId`'s context menu receive nodes instead of the webview context. */
export function registerWebviewNodes(webviewId: string, resolve: Resolver): vscode.Disposable {
  resolvers.set(webviewId, resolve);
  return new vscode.Disposable(() => {
    if (resolvers.get(webviewId) === resolve) resolvers.delete(webviewId);
  });
}

/**
 * Commands run from a webview's context menu receive VS Code's webview context ({ webview, … }) instead of a tree
 * node; swap it for the node (and the selection it is part of), so every command works unchanged.
 */
export function fromWebview(args: unknown[]): unknown[] {
  const [first] = args;
  if (first && typeof first === 'object' && typeof (first as WebviewContext).webview === 'string') {
    const resolve = resolvers.get((first as WebviewContext).webview);
    return resolve?.(first as WebviewContext) ?? [];
  }
  return args;
}
