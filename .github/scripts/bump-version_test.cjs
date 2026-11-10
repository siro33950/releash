const assert = require('node:assert/strict');
const { test } = require('node:test');
const { execFileSync } = require('node:child_process');
const { mkdtempSync, mkdirSync, writeFileSync, readFileSync, rmSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');

function bump(argument) {
  const root = mkdtempSync(join(tmpdir(), 'bump-'));
  try {
    mkdirSync(join(root, 'clients/macos'), { recursive: true });
    writeFileSync(join(root, 'Cargo.toml'), '[workspace]\nmembers = ["server"]\n\n[workspace.package]\nversion = "0.4.20"\n');
    writeFileSync(join(root, 'Cargo.lock'), ['releashd', 'releash', 'unrelated'].map(name => `[[package]]\nname = "${name}"\nversion = "0.4.20"\n`).join(''));
    writeFileSync(join(root, 'clients/macos/project.yml'), "settings:\n    MARKETING_VERSION: '0.4.20'\n");
    execFileSync(process.execPath, [join(__dirname, 'bump-version.cjs'), argument], { cwd: root, stdio: 'pipe' });
    return ['Cargo.toml', 'Cargo.lock', 'clients/macos/project.yml'].map(name => readFileSync(join(root, name), 'utf8'));
  } finally { rmSync(root, { recursive: true }); }
}

test('test_版上げ_指定の上げ幅で次の版を各配布物へ同期する', () => {
  for (const [argument, expected] of [['patch', '0.4.21'], ['minor', '0.5.0'], ['major', '1.0.0'], ['0.4.30', '0.4.30']]) {
    // When
    const [cargo, lock, project] = bump(argument);
    // Then
    assert.match(cargo, new RegExp(`version = "${expected}"`));
    assert.equal((lock.match(new RegExp(`version = "${expected}"`, 'g')) || []).length, 2);
    assert.match(lock, /name = "unrelated"\nversion = "0.4.20"/);
    assert.match(project, new RegExp(`MARKETING_VERSION: '${expected}'`));
  }
});

test('test_版上げ_上げ幅でもX.Y.Zでもない指定を拒否する', () => {
  assert.throws(() => bump('next'));
});
