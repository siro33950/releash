const assert = require('node:assert/strict');
const { test } = require('node:test');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { main, nextVersion } = require('./nightly-version.cjs');

test('test_配布版_接尾辞を認めずpatchを上げる', () => {
  // Given / When / Then
  assert.equal(nextVersion('0.4.20'), '0.4.21');
  assert.equal(nextVersion('1.2.99'), '1.2.100');
  assert.throws(() => nextVersion('0.4.20-nightly.1'));
});


for (const scenario of ['new', 'retry', 'release-head', 'failed-ci', 'draft']) {
  test(`test_nightly版確定_${scenario}`, async () => {
    // Given
    const root = mkdtempSync(join(tmpdir(), 'nightly-'));
    const previous = { ...process.env };
    process.env.GITHUB_OUTPUT = join(root, 'output');
    process.env.GITHUB_REPOSITORY = 'owner/repo';
    process.env.GITHUB_RUN_ID = '42';
    const calls = [];
    const run = (...args) => {
      calls.push(args);
      if (args[0] === 'gh' && args[1] === 'api') {
        const path = args.at(-1);
        if (path.includes('/commits?')) return JSON.stringify([[{ sha: 'original', commit: { message: scenario === 'retry' ? 'release: v0.4.21\n\nNightly-Run: 42' : scenario === 'release-head' ? 'release: v0.4.21\n\nNightly-Run: 41' : 'fix: change' } }]]);
        if (path.includes('/releases?')) return JSON.stringify([[{ draft: false, tag_name: 'v0.4.20-nightly.20261105.1' }, ...(scenario === 'draft' ? [{ draft: true, tag_name: 'v0.4.21-nightly.20261106.1' }] : [])]]);
        return JSON.stringify({ workflow_runs: [{ status: 'completed', conclusion: scenario === 'failed-ci' ? 'failure' : 'success' }] });
      }
      if (args[1] === 'rev-parse') return 'bumped';
      return '';
    };
    try {
      // When
      if (scenario === 'failed-ci') await assert.rejects(main(run), /CI has not succeeded/);
      else await main(run);
      // Then
      const bump = calls.find(args => args[0] === 'node');
      const push = calls.find(args => args[1] === 'push');
      if (['new', 'draft'].includes(scenario)) {
        assert.deepEqual(bump, ['node', '.github/scripts/bump-version.cjs', '0.4.21']);
        assert.ok(push);
        assert.equal(calls.some(args => args.includes('--cleanup-tag')), scenario === 'draft');
        assert.equal(readFileSync(process.env.GITHUB_OUTPUT, 'utf8'), 'sha=bumped\n');
      } else {
        assert.equal(bump, undefined);
        assert.equal(push, undefined);
        if (scenario !== 'failed-ci') assert.equal(readFileSync(process.env.GITHUB_OUTPUT, 'utf8'), 'sha=original\n');
      }
    } finally {
      process.env = previous;
      rmSync(root, { recursive: true });
    }
  });
}
