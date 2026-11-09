const assert = require('node:assert/strict');
const { test } = require('node:test');
const { execFileSync } = require('node:child_process');
const { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');

test('test_版同期_Nxの結果を各配布物へ同期する', () => {
  // Given
  const root = mkdtempSync(join(tmpdir(), 'bump-'));
  try {
    mkdirSync(join(root, 'clients/desktop'), { recursive: true });
    mkdirSync(join(root, 'clients/macos'), { recursive: true });
    writeFileSync(join(root, 'clients/desktop/package.json'), '{"version":"0.4.20"}');
    writeFileSync(join(root, 'clients/desktop/tauri.conf.json'), '{"version":"0.4.20","productName":"Releash"}');
    writeFileSync(join(root, 'Cargo.toml'), '[workspace.package]\nversion = "0.4.20"\n');
    writeFileSync(join(root, 'Cargo.lock'), ['releashd', 'releash', 'releash-desktop', 'unrelated'].map(name => `[[package]]\nname = "${name}"\nversion = "0.4.20"\n`).join(''));
    writeFileSync(join(root, 'clients/macos/project.yml'), "settings:\n    MARKETING_VERSION: '0.4.20'\n");
    writeFileSync(join(root, 'nx'), `#!/usr/bin/env node
const fs = require('node:fs');
require('node:assert/strict').deepEqual(process.argv.slice(2), ['release', 'version', 'patch']);
fs.writeFileSync('clients/desktop/package.json', JSON.stringify({version: '0.4.21'}));
`, { mode: 0o755 });
    // When
    execFileSync(process.execPath, [join(__dirname, 'bump-version.cjs'), 'patch'], { cwd: root });
    // Then
    assert.equal(readFileSync(join(root, 'clients/desktop/tauri.conf.json'), 'utf8'), '{\n\t"version": "0.4.21",\n\t"productName": "Releash"\n}\n');
    assert.match(readFileSync(join(root, 'Cargo.toml'), 'utf8'), /version = "0.4.21"/);
    const lock = readFileSync(join(root, 'Cargo.lock'), 'utf8');
    assert.equal((lock.match(/version = "0.4.21"/g) || []).length, 3);
    assert.match(lock, /name = "unrelated"\nversion = "0.4.20"/);
    assert.match(readFileSync(join(root, 'clients/macos/project.yml'), 'utf8'), /MARKETING_VERSION: '0.4.21'/);
  } finally { rmSync(root, { recursive: true }); }
});
