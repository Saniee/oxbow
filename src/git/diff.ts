// Unified diffs for display (the SlothGit tab's detail pane): files, hunks and lines with their line numbers.
// Pure, so it can be unit tested. git/patch.ts parses single-file diffs for rewriting; this one only reads.

export interface DisplayLine {
  kind: ' ' | '-' | '+' | '\\';
  text: string;
  /** Line number on the old side (context and removed lines). */
  old?: number;
  /** Line number on the new side (context and added lines). */
  new?: number;
}

export interface DisplayHunk {
  /** The @@ line. */
  header: string;
  lines: DisplayLine[];
}

export interface DisplayFile {
  path: string;
  oldPath?: string;
  /** A (added), D (deleted), R (renamed), C (copied) or M. */
  status: string;
  binary: boolean;
  /** Mode changes and other extended header lines worth showing. */
  notes: string[];
  added: number;
  removed: number;
  hunks: DisplayHunk[];
}

const HUNK_HEADER = /^@@ -(\d+)(?:,(\d+))? \+(\d+)(?:,(\d+))? @@/;

/** git's C-style quoting of unusual paths ("a/t\303\251st") back to text. */
function unquotePath(raw: string): string {
  if (!raw.startsWith('"')) return raw;
  const bytes: number[] = [];
  const escapes: Record<string, number> = { n: 10, t: 9, '"': 34, '\\': 92, a: 7, b: 8, f: 12, r: 13, v: 11 };
  for (let i = 1; i < raw.length - 1; i++) {
    const c = raw[i];
    if (c !== '\\') {
      bytes.push(...Buffer.from(c, 'utf8'));
      continue;
    }
    const next = raw[++i];
    if (/[0-7]/.test(next)) {
      bytes.push(parseInt(raw.slice(i, i + 3), 8));
      i += 2;
    } else {
      bytes.push(escapes[next] ?? next.charCodeAt(0));
    }
  }
  return Buffer.from(bytes).toString('utf8');
}

/** The path from a ---/+++ line, without its a/ or b/ prefix; undefined for /dev/null. */
function sidePath(line: string): string | undefined {
  const raw = unquotePath(line.slice(4).replace(/\t.*$/, ''));
  if (raw === '/dev/null') return undefined;
  return raw.replace(/^[ab]\//, '');
}

/** Parses `git diff` / `git show` output (any number of files). */
export function parseDiff(text: string): DisplayFile[] {
  const files: DisplayFile[] = [];
  let file: DisplayFile | undefined;
  let hunk: DisplayHunk | undefined;
  let oldLine = 0;
  let newLine = 0;
  for (const line of text.split('\n')) {
    if (line.startsWith('diff --git ')) {
      // Fallback path from the header, for diffs without ---/+++ lines (binary files, mode-only changes).
      const m = /^diff --git (?:"?a\/)(.*?)"? (?:"?b\/)(.*?)"?$/.exec(line);
      const unquote = (p: string) => (line.includes('"') ? unquotePath(`"${p}"`) : p);
      file = { path: m ? unquote(m[2]) : line.slice(11), status: 'M', binary: false, notes: [], added: 0, removed: 0, hunks: [] };
      if (m && m[1] !== m[2]) file.oldPath = unquote(m[1]);
      files.push(file);
      hunk = undefined;
      continue;
    }
    if (!file) continue;
    const header = HUNK_HEADER.exec(line);
    if (header) {
      hunk = { header: line, lines: [] };
      file.hunks.push(hunk);
      oldLine = parseInt(header[1], 10);
      newLine = parseInt(header[3], 10);
      continue;
    }
    if (!hunk) {
      if (line.startsWith('new file mode')) file.status = 'A';
      else if (line.startsWith('deleted file mode')) file.status = 'D';
      else if (line.startsWith('rename from ')) {
        file.status = 'R';
        file.oldPath = unquotePath(line.slice(12));
      } else if (line.startsWith('rename to ')) file.path = unquotePath(line.slice(10));
      else if (line.startsWith('copy from ')) {
        file.status = 'C';
        file.oldPath = unquotePath(line.slice(10));
      } else if (line.startsWith('copy to ')) file.path = unquotePath(line.slice(8));
      else if (line.startsWith('old mode ') || line.startsWith('new mode ')) file.notes.push(line);
      else if (line.startsWith('Binary files') || line.startsWith('GIT binary patch')) file.binary = true;
      else if (line.startsWith('--- ')) {
        const old = sidePath(line);
        if (old && old !== file.path && file.status !== 'A') file.oldPath ??= old;
      } else if (line.startsWith('+++ ')) {
        const now = sidePath(line);
        if (now) file.path = now;
      }
      continue;
    }
    const kind = line[0];
    const text = line.slice(1);
    if (kind === ' ') hunk.lines.push({ kind, text, old: oldLine++, new: newLine++ });
    else if (kind === '-') {
      hunk.lines.push({ kind, text, old: oldLine++ });
      file.removed++;
    } else if (kind === '+') {
      hunk.lines.push({ kind, text, new: newLine++ });
      file.added++;
    } else if (kind === '\\') hunk.lines.push({ kind, text: line.slice(2) });
  }
  for (const f of files) {
    if (f.oldPath === f.path) delete f.oldPath;
    if (f.status === 'M' && f.oldPath) f.status = 'R';
  }
  return files;
}

/**
 * Cuts `files` down to about `maxLines` diff lines, so a huge commit doesn't freeze the page. Whole hunks are kept
 * while they fit; the rest are counted in `omitted`.
 */
export function limitDiff(files: DisplayFile[], maxLines: number): { files: DisplayFile[]; omitted: number } {
  let budget = maxLines;
  let omitted = 0;
  const kept = files.map((f) => {
    const hunks: DisplayHunk[] = [];
    for (const h of f.hunks) {
      if (budget > 0) {
        hunks.push(h);
        budget -= h.lines.length + 1;
      } else omitted += h.lines.length;
    }
    return { ...f, hunks };
  });
  return { files: kept, omitted };
}
