import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdtempSync, readFileSync, writeFileSync, rmSync } from 'node:fs';
import { createRequire } from 'node:module';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
const require = createRequire(new URL('../../.nx/installation/package.json', import.meta.url));
const { parse } = require('yaml');
const { jobs } = parse(readFileSync('.github/workflows/nightly.yml', 'utf8'));

test('both builds and publication are gated on E2E, cleanup waits for both uploads', () => {
  function ancestors(name) {
    const needs = jobs[name].needs;
    return [needs ?? []].flat().flatMap(parent => [parent, ...ancestors(parent)]);
  }
  for (const name of ['create-release', 'release', 'swift-release', 'publish']) {
    assert.ok(ancestors(name).includes('macos-e2e'), name);
    assert.ok(ancestors(name).includes('validation'), name);
    assert.equal(jobs[name].if, undefined);
  }
  for (const name of ['release', 'swift-release']) {
    assert.ok(ancestors('publish').includes(name));
    assert.ok(ancestors('cleanup').includes(name));
  }
});

test('tee preserves failed E2E exit status and its log', t => {
  const directory = mkdtempSync(join(tmpdir(), 'releash-nightly-'));
  t.after(() => rmSync(directory, { recursive: true, force: true }));
  writeFileSync(join(directory, 'nx'), '#!/bin/bash\necho failing-test\nexit 65\n', { mode: 0o755 });
  const step = jobs['macos-e2e'].steps.find(step => step.name === 'Run all regression tests');
  assert.equal(step.shell, 'bash');
  const result = spawnSync('bash', ['--noprofile', '--norc', '-eo', 'pipefail', '-c', step.run], {
    cwd: directory, env: { ...process.env, RUNNER_TEMP: directory, XCODE_XCCONFIG_FILE: join(directory, 'test.xcconfig') },
  });
  assert.equal(result.status, 65, result.stderr.toString());
  assert.match(readFileSync(join(directory, 'e2e-evidence/test.log'), 'utf8'), /failing-test/);
});

test('publication refuses missing or empty Swift/Tauri assets', async () => {
  const script = jobs.publish.steps[0].with.script;
  const run = new (Object.getPrototypeOf(async () => {}).constructor)('github', 'context', 'process', script);
  const names = ['Releash_Preview_0.4.20_universal.zip', 'Releash_0.4.20_universal.dmg', 'Releash_0.4.20_universal.app.tar.gz', 'Releash_0.4.20_universal.app.tar.gz.sig', 'latest.json'];
  for (const missing of [...names, null]) {
    let published = false;
    const github = { paginate: async () => names.map(name => ({ name, size: name === missing ? 0 : 10 })),
      rest: { repos: { listReleaseAssets() {}, updateRelease: async () => { published = true; } } } };
    const invoke = () => run(github, { repo: {} }, { env: { RELEASE_ID: '1' } });
    if (missing) await assert.rejects(invoke, /Missing release asset/);
    else await invoke();
    assert.equal(published, missing === null);
  }
});
