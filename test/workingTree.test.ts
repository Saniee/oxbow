import assert from 'node:assert/strict';
import { execFileSync } from 'node:child_process';
import * as fs from 'node:fs';
import * as path from 'node:path';
import { test } from 'node:test';
import { GitOps } from '../src/git/operations';
import { Git } from '../src/git/runner';
import { getStatus, isStaged, isUnstaged, workingDiff } from '../src/git/workingTree';
import { concurrency, git, newRepo, writeFile } from './repo';

/** main with a.txt and old.txt committed, then: a.txt modified (staged and again unstaged), old.txt renamed to
 * new.txt (staged), b.txt added (staged), u.txt untracked. */
function setup() {
  const dir = newRepo('working-tree', (d) => {
    writeFile(d, 'a.txt', 'a1\n');
    writeFile(d, 'old.txt', 'same\nlines\nhere\n');
    git(d, 'add', '.');
    git(d, 'commit', '-q', '-m', 'c1');
  });
  writeFile(dir, 'a.txt', 'a2\n');
  git(dir, 'add', 'a.txt');
  writeFile(dir, 'a.txt', 'a3\n');
  git(dir, 'mv', 'old.txt', 'new.txt');
  writeFile(dir, 'b.txt', 'b\n');
  git(dir, 'add', 'b.txt');
  writeFile(dir, 'u.txt', 'u\n');
  const g = new Git('git', dir);
  // Not git(): it trims, which would drop the first line's leading space.
  const status = () => execFileSync('git', ['status', '--porcelain'], { cwd: dir, encoding: 'utf8' }).trimEnd();
  return { dir, g, ops: new GitOps(g, 'true'), status };
}

const entry = async (g: Git, file: string) => (await getStatus(g)).find((e) => e.path === file)!;

test('status entries and their diffs', { concurrency }, async () => {
  const { g } = setup();
  const entries = await getStatus(g);
  assert.deepEqual(
    entries.map((e) => [e.x + e.y, e.path, e.oldPath, isStaged(e), isUnstaged(e)]),
    [
      ['MM', 'a.txt', undefined, true, true],
      ['A ', 'b.txt', undefined, true, false],
      ['R ', 'new.txt', 'old.txt', true, false],
      ['??', 'u.txt', undefined, false, true],
    ],
  );
  assert.match(await workingDiff(g, entries[0], true), /-a1\n\+a2/);
  assert.match(await workingDiff(g, entries[0], false), /-a2\n\+a3/);
  assert.match(await workingDiff(g, entries[3], false), /^\+u$/m);
  assert.match(await workingDiff(g, entries[2], true), /rename from old\.txt/);
});

test('stage, unstage and discard single files', { concurrency }, async () => {
  const { dir, g, ops, status } = setup();
  await ops.stage([await entry(g, 'u.txt')]);
  await ops.unstage([await entry(g, 'a.txt'), await entry(g, 'new.txt')]);
  assert.equal(status(), [' M a.txt', 'A  b.txt', ' D old.txt', 'A  u.txt', '?? new.txt'].join('\n'));
  await ops.discard([await entry(g, 'a.txt')]);
  assert.equal(fs.readFileSync(path.join(dir, 'a.txt'), 'utf8'), 'a1\n');
  await ops.discard([await entry(g, 'b.txt'), await entry(g, 'new.txt')]);
  assert.ok(!fs.existsSync(path.join(dir, 'b.txt')) && !fs.existsSync(path.join(dir, 'new.txt')));
  assert.equal(status(), [' D old.txt', 'A  u.txt'].join('\n'));
});

test('discard a staged rename brings the old file back', { concurrency }, async () => {
  const { dir, g, ops } = setup();
  await ops.discard([await entry(g, 'new.txt')]);
  assert.ok(fs.existsSync(path.join(dir, 'old.txt')) && !fs.existsSync(path.join(dir, 'new.txt')));
});

test('discard only the unstaged part keeps what is staged', { concurrency }, async () => {
  const { dir, g, ops } = setup();
  await ops.discard([await entry(g, 'a.txt'), await entry(g, 'u.txt')], true);
  assert.equal(fs.readFileSync(path.join(dir, 'a.txt'), 'utf8'), 'a2\n');
  assert.ok(!fs.existsSync(path.join(dir, 'u.txt')));
  assert.equal((await entry(g, 'a.txt')).x + (await entry(g, 'a.txt')).y, 'M ');
});

test('stage all, unstage all, discard all', { concurrency }, async () => {
  const { g, ops, status } = setup();
  await ops.stageAll();
  assert.ok((await getStatus(g)).every((e) => isStaged(e) && !isUnstaged(e)));
  await ops.unstageAll();
  assert.ok((await getStatus(g)).every((e) => !isStaged(e)));
  await ops.discardAll();
  assert.equal(status(), '');
});

test('unstage before the first commit', { concurrency }, async () => {
  const dir = newRepo('empty');
  writeFile(dir, 'x.txt', 'x\n');
  git(dir, 'add', 'x.txt');
  const g = new Git('git', dir);
  await new GitOps(g, 'true').unstage([await entry(g, 'x.txt')]);
  assert.equal(git(dir, 'status', '--porcelain'), '?? x.txt');
});
