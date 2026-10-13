import { areDiffTargetsEqual } from '@pierre/diffs';
import test from 'node:test';
import assert from 'node:assert/strict';
import { metadata, renderType } from './metadata.js';

test('server hunks retain full contents for expansion, including added and deleted files', () => {
  for (const [original, modified, patch] of [
    ['before\na\na\na\na\na\na\na\na\na\n', 'after\na\na\na\na\na\na\na\na\na\n', '--- a/file\n+++ b/file\n@@ -1,1 +1,1 @@\n-before\n+after\n'],
    ['', 'new\n', '--- a/file\n+++ b/file\n@@ -0,0 +1,1 @@\n+new\n'],
    ['old\n', '', '--- a/file\n+++ b/file\n@@ -1,1 +0,0 @@\n-old\n'],
  ]) {
    const diff = metadata({ path: 'file', version: 1, original, modified, patch });
    assert.equal(diff.isPartial, false);
    assert.equal(diff.additionLines.join(''), modified);
    assert.equal(diff.deletionLines.join(''), original);
    assert.equal(diff.hunks.length, 1);
  }
});

test('file mode and an unchanged diff use the virtualized file renderer', () => {
  assert.equal(renderType({ mode: 'file', patch: 'server patch' }), 'file');
  assert.equal(renderType({ mode: 'diff', patch: '' }), 'file');
  assert.equal(renderType({ mode: 'diff', patch: 'server patch' }), 'diff');
});

test('changed contents with the same repository version invalidate the library render target', () => {
  const before = { path: 'file', version: 1, original: 'old\n', modified: 'one\n', patch: '--- a/file\n+++ b/file\n@@ -1,1 +1,1 @@\n-old\n+one\n' };
  const after = { ...before, modified: 'two\n', patch: before.patch.replace('+one', '+two') };
  assert.equal(areDiffTargetsEqual(metadata(before), metadata(after)), false);
});
