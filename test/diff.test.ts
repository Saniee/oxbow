import assert from 'node:assert/strict';
import { test } from 'node:test';
import { limitDiff, parseDiff } from '../src/git/diff';
import { parseStatus } from '../src/git/workingTree';

const DIFF = [
  'diff --git a/src/a.ts b/src/a.ts',
  'index 1111111..2222222 100644',
  '--- a/src/a.ts',
  '+++ b/src/a.ts',
  '@@ -1,3 +1,3 @@ function x()',
  ' keep',
  '-old',
  '+new',
  ' tail',
  '\\ No newline at end of file',
  'diff --git a/new.txt b/new.txt',
  'new file mode 100644',
  'index 0000000..3333333',
  '--- /dev/null',
  '+++ b/new.txt',
  '@@ -0,0 +1,2 @@',
  '+one',
  '+two',
  'diff --git a/old name.txt b/new name.txt',
  'similarity index 90%',
  'rename from old name.txt',
  'rename to new name.txt',
  'diff --git a/img.png b/img.png',
  'deleted file mode 100644',
  'Binary files a/img.png and /dev/null differ',
  '',
].join('\n');

test('parseDiff: files, statuses, hunks and line numbers', () => {
  const files = parseDiff(DIFF);
  assert.deepEqual(
    files.map((f) => [f.status, f.path, f.oldPath, f.binary, f.added, f.removed]),
    [
      ['M', 'src/a.ts', undefined, false, 1, 1],
      ['A', 'new.txt', undefined, false, 2, 0],
      ['R', 'new name.txt', 'old name.txt', false, 0, 0],
      ['D', 'img.png', undefined, true, 0, 0],
    ],
  );
  assert.deepEqual(files[0].hunks[0].lines, [
    { kind: ' ', text: 'keep', old: 1, new: 1 },
    { kind: '-', text: 'old', old: 2 },
    { kind: '+', text: 'new', new: 2 },
    { kind: ' ', text: 'tail', old: 3, new: 3 },
    { kind: '\\', text: 'No newline at end of file' },
  ]);
  assert.equal(files[1].hunks[0].lines[0].new, 1);
});

test('parseDiff: quoted paths', () => {
  const [file] = parseDiff('diff --git "a/t\\303\\251st.txt" "b/t\\303\\251st.txt"\n--- "a/t\\303\\251st.txt"\n+++ "b/t\\303\\251st.txt"\n@@ -1 +1 @@\n-a\n+b\n');
  assert.equal(file.path, 'tést.txt');
  assert.equal(file.oldPath, undefined);
});

test('limitDiff keeps whole hunks up to the budget', () => {
  const files = parseDiff(DIFF);
  const { files: kept, omitted } = limitDiff(files, 3);
  assert.equal(kept[0].hunks.length, 1);
  assert.equal(kept[1].hunks.length, 0);
  assert.equal(omitted, 2);
});

test('parseStatus: renames, untracked and conflicts first', () => {
  const entries = parseStatus(['M  b.txt', 'R  new.txt', 'old.txt', '?? z.txt', 'UU a.txt', ' D c.txt', ''].join('\0'));
  assert.deepEqual(
    entries.map((e) => `${e.x}${e.y} ${e.path}${e.oldPath ? ` <- ${e.oldPath}` : ''}`),
    ['UU a.txt', 'M  b.txt', ' D c.txt', 'R  new.txt <- old.txt', '?? z.txt'],
  );
});
