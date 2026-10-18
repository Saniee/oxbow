import assert from 'node:assert/strict';
import { test } from 'node:test';
import { CommandLog, formatCommand, isReadOnly } from '../src/git/commandLog';
import { Git } from '../src/git/runner';
import { git, newRepo, writeFile } from './repo';

test('read-only git commands are told apart from ones that change things', () => {
  for (const args of [
    ['log', '-1'],
    ['-c', 'core.quotePath=false', 'diff', '--cached'],
    ['rev-parse', 'HEAD'],
    ['stash', 'list'],
    ['remote', '-v'],
    ['branch', '--merged', 'HEAD'],
    ['config', '--get', 'user.name'],
    ['tag'],
    ['worktree', 'list', '--porcelain'],
  ]) {
    assert.ok(isReadOnly(args), args.join(' '));
  }
  for (const args of [
    ['commit', '-m', 'x'],
    ['rebase', '--interactive', 'HEAD~2'],
    ['branch', '-m', 'a', 'b'],
    ['branch', 'new', 'abc'],
    ['tag', 'v1', 'HEAD'],
    ['remote', 'add', 'o', 'url'],
    ['stash', 'push'],
    ['config', 'user.name', 'x'],
    ['-c', 'x=y', 'add', '-A'],
  ]) {
    assert.ok(!isReadOnly(args), args.join(' '));
  }
});

test('formatCommand quotes what a shell would need quoted', () => {
  assert.equal(formatCommand(['commit', '-m', "it's done", '--', 'a b.txt']), `git commit -m 'it'\\''s done' -- 'a b.txt'`);
  assert.equal(formatCommand(['rebase', '--onto', 'origin/main', 'HEAD~2']), 'git rebase --onto origin/main HEAD~2');
});

test('the log groups an action’s mutating commands under it', async () => {
  const dir = newRepo('command-log', (d) => {
    writeFile(d, 'a.txt', 'a\n');
    git(d, 'add', '.');
    git(d, 'commit', '-q', '-m', 'c1');
  });
  const g = new Git('git', dir);
  const log = new CommandLog(() => dir);
  let changes = 0;
  log.onDidChange(() => changes++);
  const entry = log.start('Tagging…');
  await g.exec(['rev-parse', 'HEAD']);
  await g.exec(['tag', 'v1']);
  log.finish(entry, true);
  // Outside an action, a mutating command gets an entry of its own; other repositories are ignored.
  await g.exec(['tag', 'v2']);
  await new Git('git', process.cwd()).exec(['rev-parse', 'HEAD']).catch(() => undefined);
  log.dispose();
  await g.exec(['tag', 'v3']);
  assert.deepEqual(
    log.entries.map((e) => [e.title, e.status, e.commands]),
    [
      ['git tag', 'done', ['git tag v2']],
      ['Tagging', 'done', ['git tag v1']],
    ],
  );
  assert.ok(changes >= 3);
});
