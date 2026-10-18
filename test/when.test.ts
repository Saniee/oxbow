import assert from 'node:assert/strict';
import { test } from 'node:test';
import { evaluate } from '../src/tab/when';

test('when clauses: keys, negation, equality and regex', () => {
  const values = { section: 'commits', item: 'commit:head:merge', 'slothgit.hasPatch': true, 'config.slothgit.simpleMode': false };
  assert.equal(evaluate(undefined, values), true);
  assert.equal(evaluate('slothgit.hasPatch', values), true);
  assert.equal(evaluate('!slothgit.hasPatch', values), false);
  assert.equal(evaluate('!config.slothgit.simpleMode', values), true);
  assert.equal(evaluate('unknown.key', values), false);
  assert.equal(evaluate("section == 'commits'", values), true);
  assert.equal(evaluate('section == commits', values), true);
  assert.equal(evaluate('section != commits', values), false);
  assert.equal(evaluate('item =~ /^commit(?!.*:other)/', values), true);
  assert.equal(evaluate('item =~ /^commit(?!.*:(head|other))/', values), false);
  assert.equal(evaluate('item =~ /^(commit|reflog|refcommit)/ && !config.slothgit.simpleMode', values), true);
  assert.equal(evaluate('section == branches || item =~ /^commit/', values), true);
  assert.equal(evaluate('section == branches || item == stash', values), false);
});

test('when clauses: || inside a regex is not an alternative', () => {
  assert.equal(evaluate('item =~ /^(a||b)$/', { item: 'b' }), true);
  assert.equal(evaluate('item =~ /^x\\/y$/ && ok', { item: 'x/y', ok: true }), true);
});
