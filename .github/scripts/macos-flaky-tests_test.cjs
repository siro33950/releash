const assert = require('node:assert/strict');
const { test } = require('node:test');
const { flakyTests, report } = require('./macos-flaky-tests.cjs');

const testCase = (name, result, runs) => ({
  nodeType: 'Test Case', nodeIdentifier: `Suite/${name}`, name, result,
  children: runs.map(([runName, runResult, message]) => ({
    nodeType: 'Repetition', name: runName, result: runResult,
    children: message ? [{ nodeType: 'Failure Message', name: message }] : [],
  })),
});
const results = {
  testNodes: [{
    nodeType: 'Test Plan', name: 'Plan', children: [{
      nodeType: 'Unit test bundle', name: 'ReleashIntegrationTests', children: [{
        nodeType: 'Test Suite', name: 'Suite', children: [
          testCase('testFlaky()', 'Passed', [['First Run', 'Failed', 'focus timed out'], ['Retry 1', 'Passed']]),
          testCase('testBroken()', 'Failed', [['First Run', 'Failed', 'a'], ['Retry 1', 'Failed', 'b']]),
          testCase('testRerun()', 'Passed', [['First Run', 'Passed'], ['Retry 1', 'Passed']]),
          testCase('testOnce()', 'Passed', []),
        ],
      }],
    }],
  }],
};

test('test_不安定なテストの判定_再試行で通ったテストだけを返す', () => {
  // When
  const found = flakyTests(results);
  // Then
  assert.deepEqual(found, [{ test: 'ReleashIntegrationTests/Suite/testFlaky()', failure: 'focus timed out' }]);
});

for (const scenario of ['pull_request', 'new', 'existing']) {
  test(`test_不安定なテストの記録_${scenario}`, async () => {
    // Given
    const warnings = [];
    const created = [];
    const comments = [];
    const marker = '<!-- macos-flaky:';
    const github = {
      rest: {
        issues: {
          listForRepo: 'issues',
          create: async args => { created.push(args); return { data: { number: 7 } }; },
          createComment: async args => { comments.push(args); },
        },
      },
      paginate: async () => (scenario === 'existing'
        ? [{ number: 3, body: `${marker}${require('node:crypto').createHash('sha256').update('B/T/t()').digest('hex')} -->` }]
        : []),
    };
    const context = {
      eventName: scenario === 'pull_request' ? 'pull_request' : 'push', sha: 'abc', runId: 1,
      serverUrl: 'https://github.com', repo: { owner: 'o', repo: 'r' },
    };
    const core = { warning: message => warnings.push(message), info: () => {} };
    // When
    await report({ github, context, core, tests: [{ test: 'B/T/t()', failure: 'x' }], artifactUrl: 'https://artifact' });
    // Then
    assert.equal(warnings.length, scenario === 'pull_request' ? 1 : 0);
    assert.equal(created.length, scenario === 'new' ? 1 : 0);
    assert.equal(comments.length, scenario === 'existing' ? 1 : 0);
    if (scenario === 'new') {
      assert.equal(created[0].title, '[Flaky] B/T/t()');
      assert.match(created[0].body, /commit: abc/);
      assert.match(created[0].body, /https:\/\/artifact/);
    }
    if (scenario === 'existing') assert.equal(comments[0].issue_number, 3);
  });
}
