const assert = require('node:assert/strict');
const { test } = require('node:test');
const { readFileSync } = require('node:fs');
const { join } = require('node:path');
const { parse } = require('../../.nx/installation/node_modules/yaml');
const workflow = name => parse(readFileSync(join(__dirname, '../workflows', `${name}.yml`), 'utf8'));
const AsyncFunction = Object.getPrototypeOf(async function () {}).constructor;

test('test_stable昇格_mainが先でも選択したnightlyのcommitをビルドと公開へ渡す', async () => {
  // Given
  const stable = workflow('stable');
  const values = {};
  const previous = process.env.NIGHTLY_TAG;
  process.env.NIGHTLY_TAG = 'v0.4.21-nightly.20261106.1';
  const github = { rest: { repos: {
    getReleaseByTag: async ({ tag }) => {
      assert.equal(tag, process.env.NIGHTLY_TAG);
      return { data: { draft: false, prerelease: true } };
    },
    getCommit: async ({ ref }) => ({ data: { sha: ref === process.env.NIGHTLY_TAG ? 'nightly-commit' : 'newer-main' } }),
  } } };
  try {
    // When
    await new AsyncFunction('github', 'context', 'core', stable.jobs.resolve.steps[0].with.script)(github, { repo: {} }, { setOutput: (key, value) => { values[key] = value; } });
    // Then
    assert.equal(values.sha, 'nightly-commit');
    const versionStep = stable.jobs['create-release'].steps.find(step => step.id === 'version');
    const releaseSHA = process.env.RELEASE_SHA;
    process.env.RELEASE_SHA = values.sha;
    try {
      const selectedGithub = { rest: { repos: {
        getCommit: async ({ ref }) => {
          assert.equal(ref, 'v0.4.21');
          return { data: { sha: values.sha } };
        },
        listReleases: 'releases',
      } }, paginate: async () => [] };
      await new AsyncFunction('github', 'context', 'core', 'require', versionStep.with.script)(
        selectedGithub, { repo: {} }, { setOutput: (key, value) => { values[key] = value; } },
        path => { assert.equal(path, 'node:fs'); return { readFileSync: () => '[workspace.package]\nversion = "0.4.21"\n' }; });
      assert.equal(values.tag, 'v0.4.21');
    } finally {
      if (releaseSHA === undefined) delete process.env.RELEASE_SHA;
      else process.env.RELEASE_SHA = releaseSHA;
    }
    for (const job of ['swift-build', 'upload', 'publish']) {
      assert.equal(stable.jobs[job].with.sha, '${{ needs.resolve.outputs.sha }}');
    }
    for (const step of stable.jobs['create-release'].steps.filter(step => step.uses?.startsWith('actions/checkout@'))) {
      assert.equal(step.with.ref, '${{ needs.resolve.outputs.sha }}');
    }
    for (const job of Object.values(workflow('swift-build').jobs)) {
      assert.equal(job.steps.find(step => step.uses?.startsWith('actions/checkout@')).with.ref, '${{ inputs.sha }}');
    }
    const script = readFileSync(join(__dirname, 'macos-release.sh'), 'utf8');
    assert.ok(script.indexOf('verify-release.py') < script.indexOf('PYSPARKLE'));
  } finally {
    if (previous === undefined) delete process.env.NIGHTLY_TAG;
    else process.env.NIGHTLY_TAG = previous;
  }
});

test('test_公開手順_両チャンネルを直列化しPages失敗を伝播し公開完了後に削除する', () => {
  // Given
  const nightly = workflow('nightly');
  const stable = workflow('stable');
  const publish = workflow('sparkle-publish').jobs.publish.steps;
  // When
  const deployment = publish.find(step => step.uses?.startsWith('actions/deploy-pages@'));
  const upload = publish.find(step => step.uses?.startsWith('actions/upload-pages-artifact@'));
  // Then
  assert.equal(upload.with.name, 'github-pages-${{ github.run_attempt }}');
  assert.equal(deployment.with.artifact_name, upload.with.name);
  assert.deepEqual(nightly.concurrency, stable.concurrency);
  assert.equal(nightly.concurrency['cancel-in-progress'], false);
  assert.ok(nightly.concurrency.group);
  assert.ok(publish.findIndex(step => step.run?.includes('sparkle-release.cjs publish')) < publish.findIndex(step => step.uses?.startsWith('actions/deploy-pages@')));
  for (const step of publish) assert.ok(!step['continue-on-error']);
  assert.ok(!workflow('sparkle-publish').jobs.publish['continue-on-error']);
  assert.equal(nightly.jobs.cleanup.if, "needs.publish.result == 'success'");
});

test('test_Release作成_共通処理はdraftの作成と再実行を両チャンネルで扱う', async () => {
  // Given
  const action = parse(readFileSync(join(__dirname, '../actions/create-release/action.yml'), 'utf8'));
  const previous = { ...process.env };
  for (const nightly of [true, false]) {
    const calls = [];
    const values = {};
    process.env.NIGHTLY = String(nightly);
    process.env.RELEASE_TAG = 'v0.4.21';
    process.env.RELEASE_SHA = 'selected';
    delete process.env.RELEASE_ID;
    const github = { rest: { repos: { createRelease: async value => { calls.push(value); return { data: { id: 42 } }; } } } };
    const create = new AsyncFunction('github', 'context', 'core', 'require', action.runs.steps[1].with.script);
    const core = { setOutput: (key, value) => { values[key] = value; } };
    try {
      // When
      await create(github, { repo: {} }, core, () => ({ readFileSync: () => 'notes' }));
      process.env.RELEASE_ID = '42';
      await create(github, { repo: {} }, core, require);
      // Then
      assert.equal(calls.length, 1);
      assert.equal(calls[0].draft, true);
      assert.equal(calls[0].prerelease, nightly);
      assert.equal(calls[0].target_commitish, 'selected');
      assert.equal(values['release-id'], '42');
    } finally { process.env = { ...previous }; }
  }
});

test('test_公開後の保持数制限_古いnightlyを削除しても両チャンネルの参照先が残る', async () => {
  // Given
  const { appcast } = require('./sparkle-release.cjs');
  const make = (patch, nightly) => ({
    id: nightly ? patch : 100, prerelease: nightly, draft: false,
    tag_name: `v0.4.${patch}${nightly ? '-nightly.20261106.1' : ''}`,
    published_at: `2026-11-${String(patch).padStart(2, '0')}`,
    body: `<!-- sparkle:${JSON.stringify({ version: `0.4.${patch}`, signature: 'a'.repeat(86) + '==', length: 10 })} -->`,
    assets: [{ name: `Releash_0.4.${patch}_universal.zip`, size: 10, browser_download_url: `https://example.org/${nightly ? 'nightly' : 'stable'}/${patch}.zip` }],
  });
  let releases = [make(10, false), ...Array.from({ length: 16 }, (_, index) => make(index + 1, true))];
  const removed = [];
  const github = { paginate: async () => releases, rest: { repos: {
    listReleases: 'releases',
    deleteRelease: async ({ release_id }) => {
      removed.push(release_id);
      releases = releases.filter(release => release.id !== release_id);
    },
  } } };
  const script = workflow('nightly').jobs.cleanup.steps[0].with.script;
  const xml = appcast(releases);
  const publishedURLs = [...xml.matchAll(/<enclosure[^>]* url="([^"]+)"/g)].map(match => match[1]);
  // When
  await new AsyncFunction('github', 'context', script)(github, { repo: {} });
  // Then
  assert.equal(releases.filter(release => release.prerelease).length, 14);
  assert.deepEqual(removed, [2, 1]);
  assert.equal(publishedURLs.length, 2);
  const remainingURLs = releases.flatMap(release => release.assets.map(asset => asset.browser_download_url));
  for (const url of publishedURLs) assert.ok(remainingURLs.includes(url), url);
  assert.ok(xml.includes('https://example.org/stable/10.zip'));
  assert.ok(xml.includes('https://example.org/nightly/16.zip'));
  for (const id of removed) assert.ok(!xml.includes(`https://example.org/nightly/${id}.zip`));
});
