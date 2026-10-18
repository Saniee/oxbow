import * as crypto from 'node:crypto';
import * as vscode from 'vscode';

export type Page = 'graph' | 'tab';

/** Where the bundled pages are: esbuild.mjs builds src/webview/<page>/main.ts into dist/webview/<page>.js and .css. */
export function webviewRoot(extensionUri: vscode.Uri): vscode.Uri {
  return vscode.Uri.joinPath(extensionUri, 'dist', 'webview');
}

/** Webview options that let a page load its bundle and nothing else from disk. */
export function webviewOptions(extensionUri: vscode.Uri): vscode.WebviewOptions {
  return { enableScripts: true, localResourceRoots: [webviewRoot(extensionUri)] };
}

/**
 * The document for a page: its markup (src/webview/<page>/body.html) with its bundled script and styles, under a
 * Content Security Policy that allows only those. Styles set from script (element.style) are not affected by it.
 */
export function pageHtml(webview: vscode.Webview, extensionUri: vscode.Uri, page: Page, title: string, body: string): string {
  const nonce = crypto.randomBytes(16).toString('base64');
  const asset = (ext: string) => webview.asWebviewUri(vscode.Uri.joinPath(webviewRoot(extensionUri), `${page}.${ext}`));
  return `<!DOCTYPE html>
<html lang="en">
<head>
<meta charset="UTF-8">
<meta http-equiv="Content-Security-Policy" content="default-src 'none'; style-src ${webview.cspSource}; font-src ${webview.cspSource}; script-src 'nonce-${nonce}';">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>${title}</title>
<link rel="stylesheet" href="${asset('css')}">
</head>
<body>
${body}
<script nonce="${nonce}" src="${asset('js')}"></script>
</body>
</html>`;
}
