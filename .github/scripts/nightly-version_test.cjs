const assert = require('node:assert/strict');
const { test } = require('node:test');
const { mkdtempSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');
const { main, nextVersion } = require('./nightly-version.cjs');

test('test_配布版_接尾辞を認めずpatchを上げる', () => {
  // Given
  const versions = ['0.4.20', '1.2.99'];
  const nightly = '0.4.20-nightly.1';
  // When
  const next = versions.map(nextVersion);
  const bumpNightly = () => nextVersion(nightly);
  // Then
  assert.deepEqual(next, ['0.4.21', '1.2.100']);
  assert.throws(bumpNightly);
});


for (const scenario of ['new', 'retry', 'release-head', 'failed-ci', 'draft', 'minor-bump', 'paginated']) {
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
        assert.ok(!path.includes('/commits?'));
        if (path.includes('/releases?')) {
          assert.deepEqual(args.slice(0, 4), ['gh', 'api', '--jq', 'map({tag_name, draft})']);
          if (scenario === 'paginated') {
            if (path.endsWith('page=1')) return JSON.stringify([
              { draft: true, tag_name: 'v0.6.1-nightly.20261106.1' },
              ...Array.from({ length: 29 }, () => ({ draft: false, tag_name: 'v0.4.20' })),
            ]);
            assert.ok(path.endsWith('page=2'));
            return JSON.stringify([
              { draft: false, tag_name: 'v0.6.0' },
              { draft: true, tag_name: 'v9.0.0-nightly.20261106.1' },
              { draft: false, tag_name: 'not-a-version' },
            ]);
          }
          return JSON.stringify([{ draft: false, tag_name: 'v0.4.20-nightly.20261105.1' }, ...(scenario === 'draft' ? [{ draft: true, tag_name: 'v0.4.21-nightly.20261106.1' }] : [])]);
        }
        return JSON.stringify({ workflow_runs: [{ status: 'completed', conclusion: scenario === 'failed-ci' ? 'failure' : 'success' }] });
      }
      if (args[1] === 'log') {
        if (args.includes('--format=%H')) return scenario === 'retry' ? 'original' : '';
        return scenario === 'release-head' ? 'release: v0.4.21\n\nNightly-Run: 41' : 'fix: change';
      }
      if (args[1] === 'rev-parse') return args[2] === 'origin/main' ? 'original' : 'bumped';
      return '';
    };
    try {
      // When
      const mainVersion = () => (scenario === 'minor-bump' ? '0.5.0' : '0.4.20');
      if (scenario === 'failed-ci') await assert.rejects(main(run, mainVersion), /CI has not succeeded/);
      else await main(run, mainVersion);
      // Then
      const bump = calls.find(args => args[0] === 'node');
      const push = calls.find(args => args[1] === 'push');
      if (['new', 'draft', 'minor-bump', 'paginated'].includes(scenario)) {
        assert.deepEqual(bump, ['node', '.github/scripts/bump-version.cjs', scenario === 'minor-bump' ? '0.5.0' : scenario === 'paginated' ? '0.6.1' : '0.4.21']);
        assert.ok(push);
        assert.equal(calls.some(args => args.includes('--cleanup-tag')), ['draft', 'paginated'].includes(scenario));
        if (scenario === 'paginated') {
          assert.deepEqual(calls.filter(args => args.includes('--cleanup-tag')).map(args => args[3]), ['v0.6.1-nightly.20261106.1']);
          assert.equal(calls.filter(args => args.at(-1).includes('/releases?')).length, 2);
        }
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

test('test_nightly版確定_過去の実行IDに完全一致するcommitをgit履歴から再利用する', async () => {
  const { execFileSync } = require('node:child_process');
  const root = mkdtempSync(join(tmpdir(), 'nightly-retry-'));
  const previous = { ...process.env };
  const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
  try {
    git('init', '-q');
    git('config', 'user.name', 'test');
    git('config', 'user.email', 'test@example.invalid');
    git('commit', '--allow-empty', '-qm', 'release: v0.4.21\n\nNightly-Run: 42');
    const original = git('rev-parse', 'HEAD');
    git('commit', '--allow-empty', '-qm', 'fix: later change\n\nNightly-Run: 420\nNot-Nightly-Run: 42\nNightly-Run: 42 extra');
    const head = git('rev-parse', 'HEAD');
    git('update-ref', 'refs/remotes/origin/main', head);
    process.env.GITHUB_OUTPUT = join(root, 'output');
    process.env.GITHUB_REPOSITORY = 'owner/repo';
    process.env.GITHUB_RUN_ID = '42';
    const run = (...args) => {
      if (args[0] === 'git') {
        assert.ok(['log', 'rev-parse'].includes(args[1]));
        return git(...args.slice(1));
      }
      assert.deepEqual(args, ['gh', 'api', `repos/owner/repo/actions/workflows/ci.yml/runs?branch=main&event=push&head_sha=${original}`]);
      return JSON.stringify({ workflow_runs: [{ conclusion: 'success' }] });
    };
    await main(run, () => { throw new Error('A retry must not determine a new version'); });
    assert.equal(readFileSync(process.env.GITHUB_OUTPUT, 'utf8'), `sha=${original}\n`);
    assert.equal(git('rev-parse', 'HEAD'), head);
  } finally {
    process.env = previous;
    rmSync(root, { recursive: true });
  }
});

for (const reason of ['manual-bump', 'failed-distribution']) {
  test(`test_nightly版確定_${reason}_同じ版でも実gitで配布commitを作る`, async () => {
    // Given
    const { execFileSync } = require('node:child_process');
    const { mkdirSync, writeFileSync } = require('node:fs');
    const root = mkdtempSync(join(tmpdir(), 'nightly-git-'));
    const previous = { ...process.env };
    const git = (...args) => execFileSync('git', args, { cwd: root, encoding: 'utf8' }).trim();
    git('init', '-q');
    git('config', 'user.name', 'test');
    git('config', 'user.email', 'test@example.invalid');
    for (const name of ['Cargo.toml', 'Cargo.lock', 'clients/desktop/package.json', 'clients/desktop/tauri.conf.json', 'clients/macos/project.yml']) {
      mkdirSync(require('node:path').dirname(join(root, name)), { recursive: true });
      writeFileSync(join(root, name), '0.4.21');
    }
    git('add', '.');
    git('commit', '-qm', reason === 'manual-bump' ? 'chore: bump' : 'fix: after failed distribution');
    const original = git('rev-parse', 'HEAD');
    git('update-ref', 'refs/remotes/origin/main', original);
    process.env.GITHUB_OUTPUT = join(root, 'output');
    process.env.GITHUB_REPOSITORY = 'owner/repo';
    process.env.GITHUB_RUN_ID = '43';
    const run = (...args) => {
      if (args[0] === 'gh') {
        const path = args.at(-1);
        assert.ok(!path.includes('/commits?'));
        if (path.includes('/releases?')) return JSON.stringify([{ draft: false, tag_name: 'v0.4.20' }]);
        return JSON.stringify({ workflow_runs: [{ conclusion: 'success' }] });
      }
      if (args[0] === 'node' || args[1] === 'push') return '';
      return git(...args.slice(1));
    };
    try {
      // When
      await main(run, () => '0.4.21');
      // Then
      assert.notEqual(git('rev-parse', 'HEAD'), original);
      assert.equal(git('diff', original, 'HEAD'), '');
      assert.match(git('log', '-1', '--format=%B'), /release: v0.4.21\n\nNightly-Run: 43/);
    } finally {
      process.env = previous;
      rmSync(root, { recursive: true });
    }
  });
}
