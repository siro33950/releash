const assert = require('node:assert/strict');
const { test } = require('node:test');
const changes = require('./nightly-changes.cjs');
for (const scenario of ['unchanged', 'new', 'failed-ci', 'retry']) {
  test(`test_nightly配布判定_${scenario}`, async () => {
    // Given
    const values = {};
    const calls = [];
    const github = {
      rest: {
        repos: {
          getCommit: async ({ ref }) => ({ data: { sha: ref === 'heads/main' || scenario === 'unchanged' ? 'head' : 'older' } }),
          listCommits: 'commits', listReleases: 'releases',
        },
        actions: { listWorkflowRuns: 'runs' },
      },
      paginate: async kind => {
        calls.push(kind);
        if (kind === 'commits') return scenario === 'retry' ? [{ sha: 'retry-sha', commit: { message: 'release: v0.4.21\n\nNightly-Run: 42' } }] : [];
        if (kind === 'releases') return [{ draft: false, prerelease: true, tag_name: 'v0.4.20-nightly.20261105.1', published_at: '2026-11-05', id: 1 }];
        return scenario === 'failed-ci' ? [] : [{ conclusion: 'success' }];
      },
    };
    const args = { github, context: { repo: { owner: 'owner', repo: 'repo' }, runId: 42, eventName: 'schedule' }, core: { setOutput: (key, value) => { values[key] = value; } } };
    // When / Then
    if (scenario === 'failed-ci') await assert.rejects(changes(args), /No successful CI/);
    else {
      await changes(args);
      assert.equal(values.run, scenario !== 'unchanged');
      assert.equal(values.sha, scenario === 'retry' ? 'retry-sha' : 'head');
    }
    if (['unchanged', 'retry'].includes(scenario)) assert.ok(!calls.includes('runs'));
  });
}
