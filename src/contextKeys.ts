import * as vscode from 'vscode';

/**
 * SlothGit's context keys, kept here as well as in VS Code: the SlothGit tab draws its own buttons and evaluates
 * their `when` clauses itself (tab/when.ts), and VS Code offers no way to read a context key back.
 */
const values = new Map<string, unknown>();
const emitter = new vscode.EventEmitter<string>();

/** Fires with the key whenever a context key changes. */
export const onDidChangeContext = emitter.event;

export function setContext(key: string, value: unknown): void {
  if (values.has(key) && values.get(key) === value) return;
  values.set(key, value);
  void vscode.commands.executeCommand('setContext', key, value);
  emitter.fire(key);
}

export function contextValues(): Record<string, unknown> {
  return Object.fromEntries(values);
}
