const assert = require('node:assert/strict');
const { test } = require('node:test');
const { appcast, metadata } = require('./sparkle-release.cjs');
const { nextVersion } = require('./nightly-version.cjs');
function release(version, nightly = false) {
  return {
    tag_name: `v${version}${nightly ? '-nightly.20261106.1' : ''}`,
    prerelease: nightly, draft: false,
    body: `<!-- sparkle:${JSON.stringify({ version, signature: 'a'.repeat(86) + '==', length: 10 })} -->`,
    assets: [{ name: `Releash_${version}_universal.zip`, size: 10, browser_download_url: 'https://example.org/update.zip' }],
  };
}
test('keeps latest stable and nightly, coalesces promotions', () => {
  let xml = appcast([release('0.4.20'), release('0.4.21', true), release('0.4.19', true)]);
  assert.equal((xml.match(/<item>/g) || []).length, 2);
  assert.match(xml, /<sparkle:channel>nightly/);
  assert.ok(!xml.includes('0.4.19'));
  xml = appcast([release('0.4.21'), release('0.4.21', true)]);
  assert.equal((xml.match(/<item>/g) || []).length, 1);
  assert.ok(!xml.includes('<sparkle:channel>'));
});
test('fails before publishing on missing signature or asset', () => {
  assert.throws(() => metadata({ ...release('0.4.21'), body: '' }));
  assert.throws(() => metadata({ ...release('0.4.21'), assets: [] }));
  assert.throws(() => metadata({ ...release('0.4.21'), tag_name: 'v0.4.22' }));
});
test('patch increment uses only X.Y.Z', () => {
  assert.equal(nextVersion('0.4.20'), '0.4.21');
  assert.equal(nextVersion('1.2.99'), '1.2.100');
  assert.throws(() => nextVersion('0.4.20-nightly.1'));
});

test('archive signature must match the public key in the application', () => {
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
    verifyArchive(file, signature, publicKey);
    writeFileSync(file, 'changed');
    assert.throws(() => verifyArchive(file, signature, publicKey));
  } finally { rmSync(root, { recursive: true }); }
});
