const assert = require('node:assert/strict');
const { test } = require('node:test');
const { appcast, metadata } = require('./sparkle-release.cjs');
function release(version, nightly = false) {
  return {
    tag_name: `v${version}${nightly ? '-nightly.20261106.1' : ''}`,
    prerelease: nightly, draft: false,
    body: `<!-- sparkle:${JSON.stringify({ version, signature: 'a'.repeat(86) + '==', length: 10 })} -->`,
    assets: [{ name: `Releash_${version}_universal.zip`, size: 10, browser_download_url: 'https://example.org/update.zip' }],
  };
}
test('test_配信候補_各チャンネルの最新を残し昇格を統合する', () => {
  // Given
  const releases = [release('0.4.20'), release('0.4.21', true), release('0.4.19', true)];
  const promoted = [release('0.4.21'), release('0.4.21', true)];
  // When
  const xml = appcast(releases);
  const promotedXML = appcast(promoted);
  // Then
  assert.equal((xml.match(/<item>/g) || []).length, 2);
  assert.match(xml, /<sparkle:channel>nightly/);
  assert.ok(!xml.includes('0.4.19'));
  assert.equal((promotedXML.match(/<item>/g) || []).length, 1);
  assert.ok(!promotedXML.includes('<sparkle:channel>'));
});
test('test_配布メタデータ_署名と成果物の欠落を拒否する', () => {
  // Given
  const item = release('0.4.21');
  // When
  const missingSignature = () => metadata({ ...item, body: '' });
  const missingAsset = () => metadata({ ...item, assets: [] });
  const mismatchedVersion = () => metadata({ ...item, tag_name: 'v0.4.22' });
  // Then
  assert.throws(missingSignature);
  assert.throws(missingAsset);
  assert.throws(mismatchedVersion);
});
test('test_署名検証_改変されたアーカイブを拒否する', () => {
  // Given
  const { generateKeyPairSync, sign } = require('node:crypto');
  const { mkdtempSync, writeFileSync, rmSync } = require('node:fs');
  const { tmpdir } = require('node:os');
  const { join } = require('node:path');
  const { verifyArchive } = require('./sparkle-release.cjs');
  const root = mkdtempSync(join(tmpdir(), 'sparkle-test-'));
  try {
    const file = join(root, 'archive.zip');
    writeFileSync(file, 'archive');
    const keys = generateKeyPairSync('ed25519');
    const publicKey = keys.publicKey.export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64');
    const signature = sign(null, Buffer.from('archive'), keys.privateKey).toString('base64');
    // When
    const verify = () => verifyArchive(file, signature, publicKey);
    // Then
    assert.doesNotThrow(verify);
    // Given
    writeFileSync(file, 'changed');
    // When
    const verifyModified = () => verifyArchive(file, signature, publicKey);
    // Then
    assert.throws(verifyModified);
  } finally { rmSync(root, { recursive: true }); }
});

test('test_配布メタデータ_版の前方一致と不正タグを拒否する', () => {
  // Given
  const item = release('0.4.2');
  for (const tag_name of ['v0.4.20', 'v0.4.21-nightly.20261106.1', 'v0.4.2-other', 'v0.4.2-nightly.20261106.0']) {
    // When
    const readMetadata = () => metadata({ ...item, tag_name });
    // Then
    assert.throws(readMetadata, /version mismatch/);
  }
  // When
  const stable = metadata(item);
  const nightly = metadata(release('0.4.2', true));
  // Then
  assert.equal(stable.version, '0.4.2');
  assert.equal(nightly.version, '0.4.2');
});

for (const [failure, nightly] of ['missing-asset', 'public-asset', 'none'].flatMap(failure => [[failure, true], [failure, false]])) {
  test(`test_公開手順_${nightly ? "nightly" : "stable"}_${failure}_失敗を伝播し同じ版で再実行できる`, () => {
    // Given
    const { main } = require('./sparkle-release.cjs');
    const { mkdtempSync, writeFileSync, readFileSync, rmSync } = require('node:fs');
    const { tmpdir } = require('node:os');
    const { join } = require('node:path');
    const root = mkdtempSync(join(tmpdir(), 'publish-'));
    const previous = { ...process.env };
    const cwd = process.cwd();
    process.chdir(root);
    process.env.GITHUB_REPOSITORY = 'owner/repo';
    process.env.RELEASE_TAG = release('0.4.21', nightly).tag_name;
    process.env.NIGHTLY = String(nightly);
    process.env.ASSETS = root;
    let target = { ...release('0.4.21', nightly), id: 2, draft: true, assets: [], body: 'notes' };
    const stable = { ...release('0.4.20'), id: 1 };
    let fail = failure;
    const calls = [];
    writeFileSync(join(root, 'Releash_0.4.21_universal.zip'), '0123456789');
    writeFileSync(join(root, 'sparkle.json'), JSON.stringify({ version: '0.4.21', signature: 'a'.repeat(86) + '==', length: 10 }));
    const run = (...args) => {
      calls.push(args);
      if (args[0] === 'curl') {
        assert.equal(target.draft, false);
        if (fail === 'public-asset') throw new Error('unavailable asset');
        return '';
      }
      if (args[1] === 'api') return JSON.stringify(args.at(-1).includes('/tags/') ? target : [[stable, target]]);
      if (args[2] === 'upload') target.assets = fail === 'missing-asset' ? [] : release('0.4.21', true).assets;
      if (args.includes('--notes-file')) target.body = readFileSync(args[args.indexOf('--notes-file') + 1], 'utf8');
      if (args.includes('--draft=false')) {
        assert.ok(readFileSync('public/appcast.xml', 'utf8').includes('0.4.21'));
        target.draft = false;
      }
      return '';
    };
    try {
      if (fail === 'missing-asset') {
        // When
        const upload = () => main('upload', run);
        // Then
        assert.throws(upload, /Missing Swift asset/);
        assert.ok(!calls.some(args => args.includes('--draft=false')));
        fail = 'none';
      }
      // When
      main('upload', run);
      if (fail === 'public-asset') {
        const publish = () => main('publish', run);
        // Then
        assert.throws(publish, /unavailable asset/);
        fail = 'none';
      }
      // When
      main('upload', run);
      main('publish', run);
      const first = readFileSync('public/appcast.xml', 'utf8');
      main('publish', run);
      // Then
      assert.equal(readFileSync('public/appcast.xml', 'utf8'), first);
      assert.ok(first.includes('0.4.21'));
      assert.equal(first.includes('0.4.20'), nightly);
      assert.ok(calls.some(args => args.includes(`--prerelease=${nightly}`) && args.includes(`--latest=${!nightly}`)));
      assert.equal(target.tag_name, process.env.RELEASE_TAG);
    } finally {
      process.chdir(cwd);
      process.env = previous;
      rmSync(root, { recursive: true });
    }
  });
}

test('test_appcast_削除済みnightlyを参照せず両チャンネルの公開順に依存しない', () => {
  // Given
  const retained = Array.from({ length: 14 }, (_, i) => release(`0.4.${i + 21}`, true));
  const stable = release('0.4.30');
  // When
  const first = appcast([stable, ...retained]);
  const second = appcast([...retained.reverse(), stable]);
  // Then
  assert.equal(first, second);
  assert.ok(first.includes('0.4.34') && first.includes('0.4.30'));
  assert.ok(!first.includes('0.4.20'));
});
