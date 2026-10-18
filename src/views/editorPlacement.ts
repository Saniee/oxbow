import * as vscode from 'vscode';

/** Whether the SlothGit tab is the editor in front. */
function tabInFront(): boolean {
  const input = vscode.window.tabGroups.activeTabGroup.activeTab?.input;
  return input instanceof vscode.TabInputWebview && input.viewType.endsWith('slothgit.tab');
}

/**
 * Where an editor opened from SlothGit goes: beside the SlothGit tab while that is in front, so the tab stays in
 * view (as lazygit keeps its panels); otherwise where VS Code would put it.
 */
export function editorColumn(): vscode.ViewColumn | undefined {
  return tabInFront() ? vscode.ViewColumn.Beside : undefined;
}

/**
 * For commands that open in the active editor group and take no column (vscode.changes): moves to the group to the
 * right of the SlothGit tab first, creating one if needed.
 */
export async function leaveTab(): Promise<void> {
  if (!tabInFront()) return;
  const groups = vscode.window.tabGroups.all;
  const active = vscode.window.tabGroups.activeTabGroup;
  const next = groups.find((g) => g.viewColumn === active.viewColumn + 1);
  await vscode.commands.executeCommand(next ? 'workbench.action.focusNextGroup' : 'workbench.action.newGroupRight');
}
