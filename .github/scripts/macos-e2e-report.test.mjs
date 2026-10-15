import assert from 'node:assert/strict';
import { mkdtempSync, mkdirSync, writeFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import report from './macos-e2e-report.cjs';

function fixture(t, summary) {
  const evidence = mkdtempSync(join(tmpdir(), 'releash-e2e-report-'));
  t.after(() => rmSync(evidence, { recursive: true, force: true }));
  if (summary) writeFileSync(join(evidence, 'summary.json'), JSON.stringify(summary));
  const created = [], comments = [], open = [], runs = [];
  const commits = [{ sha: 'change', commit: { message: 'fix: change\nbody' } }];
  const github = {
    rest: {
      actions: { listWorkflowRuns: 'runs' },
      repos: { compareCommitsWithBasehead: 'commits' },
      issues: {
        listForRepo: 'issues',
        create: async issue => { created.push(issue); return { data: { number: 7 } }; },
        createComment: async comment => { comments.push(comment); },
      },
    },
    paginate: async (endpoint, params, map) => {
      if (endpoint === 'runs') return runs;
      if (endpoint === 'issues') return open;
      assert.equal(params.basehead, 'previous...current');
      return map({ data: { commits } });
    },
  };
  const invoke = () => report({ github, evidence, context: {
    repo: { owner: 'owner', repo: 'repo' }, serverUrl: 'https://github.com', runId: 100,
  }, core: { info() {}, warning() {} }, artifactUrl: 'https://github.com/artifact', sha: 'current' });
  return { invoke, evidence, created, comments, open, runs, commits };
}
const failure = {
  targetName: 'UITests', testIdentifierString: 'Workbench/testFoo()', failureText: 'expected true',
};

test('creates one issue per failed test, with screenshots and commits since the previous successful Nightly', async t => {
  const f = fixture(t, { failedTests: 1, testFailures: [failure, failure] });
  f.runs.push({ id: 50, head_sha: 'previous' }, { id: 200, head_sha: 'future' });
  mkdirSync(join(f.evidence, 'screenshots'));
  writeFileSync(join(f.evidence, 'screenshots/manifest.json'), JSON.stringify([{
    testIdentifier: 'Workbench/testFoo()', attachments: [{ exportedFileName: 'failure.png' }],
  }]));
  await f.invoke();
  assert.equal(f.created.length, 1);
  assert.match(f.created[0].body, /screenshots\/failure.png/);
  assert.match(f.created[0].body, /change: fix: change/);
  assert.match(f.created[0].body, /expected true/);
  assert.match(f.created[0].body, /actions\/runs\/100/);
});

test('adds evidence to the existing open issue for the same test', async t => {
  const f = fixture(t, { failedTests: 1, testFailures: [failure] });
  await f.invoke();
  f.open.push({ number: 7, body: f.created[0].body });
  f.created.length = 0;
  await f.invoke();
  assert.equal(f.created.length, 0);
  assert.equal(f.comments[0].issue_number, 7);
});

test('reports runner or build failure even when no xcresult was produced', async t => {
  const f = fixture(t);
  await f.invoke();
  assert.match(f.created[0].title, /runner-or-build/);
  assert.match(f.created[0].body, /成功した Nightly はまだありません/);
});

test('reports failure even when result export left an incomplete JSON file', async t => {
  const f = fixture(t);
  writeFileSync(join(f.evidence, 'summary.json'), '{');
  await f.invoke();
  assert.match(f.created[0].title, /runner-or-build/);
});

test('keeps every commit across comments when the comparison exceeds the issue body limit', async t => {
  const f = fixture(t, { failedTests: 1, testFailures: [failure] });
  f.runs.push({ id: 50, head_sha: 'previous' });
  f.commits.splice(0, 1, ...Array.from({ length: 200 }, (_, i) => ({
    sha: `commit-${i}`, commit: { message: 'x'.repeat(600) },
  })));
  await f.invoke();
  assert.ok(f.comments.length > 0);
  const bodies = [...f.created, ...f.comments].map(item => item.body);
  assert.ok(bodies.every(body => body.length < 65536));
  for (const commit of f.commits) assert.ok(bodies.some(body => body.includes(`${commit.sha}:`)));
  assert.ok(f.comments.every(comment => comment.issue_number === 7));
});
