// Tree items (what the providers produce) as rows for the SlothGit tab's page.
import * as path from 'node:path';
import * as vscode from 'vscode';
import { parseRefs } from '../git/graph';
import { THEME_COLOR_DEFAULTS, cssVar } from '../themeColors';
import type { Node } from '../views/nodes';
import { initials } from '../views/providers';
import type { TabIcon, TabRow } from '../webview/messages';

/** A theme colour as the CSS variable VS Code defines for it in webviews, with SlothGit's own colours' defaults as fallbacks. */
export function cssColor(color: vscode.ThemeColor | undefined): string | undefined {
  const id = (color as { id?: string } | undefined)?.id;
  if (!id) return undefined;
  const fallback = THEME_COLOR_DEFAULTS[id];
  return fallback ? `var(${cssVar(id)}, var(${cssVar(fallback)}))` : `var(${cssVar(id)})`;
}

/** Markdown tooltips as plain text: no escapes, no emphasis, no $(icons). */
export function plainText(value: string | vscode.MarkdownString | undefined): string | undefined {
  if (value === undefined) return undefined;
  if (typeof value === 'string') return value;
  return value.value
    .replace(/\$\([\w-]+\)\s?/g, '')
    .replace(/\*\*(.*?)\*\*/g, '$1')
    .replace(/`([^`]*)`/g, '$1')
    .replace(/\\([\\`*_{}[\]()#+\-.!|<>])/g, '$1')
    .replace(/\n{3,}/g, '\n\n')
    .trim();
}

function icon(item: vscode.TreeItem): TabIcon | undefined {
  const theme = item.iconPath;
  return theme instanceof vscode.ThemeIcon ? { id: theme.id, color: cssColor(theme.color) } : undefined;
}

export interface RowContext {
  id: string;
  view: string;
  actions: string[];
  parent?: string;
  expandable?: boolean;
  /** Remote names, to tell remote branches from local ones in commit decorations. */
  remotes: string[];
}

export function toRow(item: vscode.TreeItem, node: Node, context: RowContext): TabRow {
  const label = typeof item.label === 'string' ? item.label : (item.label?.label ?? (item.resourceUri ? path.basename(item.resourceUri.fsPath) : ''));
  const row: TabRow = {
    id: context.id,
    label,
    highlights: typeof item.label === 'object' ? item.label.highlights : undefined,
    description: typeof item.description === 'string' ? item.description : undefined,
    tooltip: plainText(item.tooltip as string | vscode.MarkdownString | undefined),
    icon: icon(item),
    view: context.view,
    item: item.contextValue ?? node.kind,
    actions: context.actions,
    parent: context.parent,
    expandable: context.expandable,
  };
  if (node.kind === 'commit') {
    row.commit = {
      shortSha: node.shortSha,
      people: node.author ? [node.author, ...(node.coAuthors ?? []).map((p) => p.name)].map(initials).join('+') : '',
      subject: node.subject,
      refs: parseRefs(node.refs ?? [], context.remotes),
      date: node.relDate,
    };
    row.dim = node.onHead === false;
  } else if (node.kind === 'file') {
    row.status = node.pair.change.status;
  } else if (node.kind === 'change') {
    row.status = node.entry.x + node.entry.y;
  }
  return row;
}
