import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, copyFileSync, writeFileSync, readFileSync, rmSync, existsSync, realpathSync, symlinkSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import test from 'node:test';

function release(t, status, env = {}) {
    const root = realpathSync(mkdtempSync(join(tmpdir(), 'releash-signing-')));
    t.after(() => rmSync(root, { recursive: true, force: true }));
    for (const path of ['.github/scripts', 'clients/macos', 'clients/desktop', 'bin']) mkdirSync(join(root, path), { recursive: true });
    copyFileSync('.github/scripts/macos-release.sh', join(root, '.github/scripts/macos-release.sh'));
    writeFileSync(join(root, 'Cargo.toml'), '[workspace.package]\nversion = "0.4.20"\n');
    writeFileSync(join(root, 'clients/desktop/package.json'), '{"version":"0.4.20"}');
    const mock = `#!/usr/bin/env python3
import json, os, pathlib, plistlib, sys
name = pathlib.Path(sys.argv[0]).name
args = sys.argv[1:]
with open(os.environ['CALLS'], 'a') as file:
    file.write(json.dumps([name, *args]) + '\\n')
if name == 'openssl': print('temporary-password')
if name == 'security' and args == ['list-keychains', '-d', 'user']: print('"/tmp/login.keychain"')
if name == 'xcodebuild' and 'archive' in args:
    app = pathlib.Path(args[args.index('-archivePath') + 1]) / 'Products/Applications/Releash Preview.app/Contents'
    app.mkdir(parents=True)
    with (app / 'Info.plist').open('wb') as file:
        plistlib.dump({'CFBundleShortVersionString':'0.4.20', 'CFBundleIdentifier':'com.releash.macos'}, file)
if name == 'xcrun' and args[:2] == ['notarytool', 'submit']:
    assert '--wait' not in args
    print(json.dumps({'id':'submission', 'status':'In Progress'}))
if name == 'xcrun' and args[:2] == ['notarytool', 'wait']:
    assert args[2] == 'submission'
    failed = sum(1 for line in open(os.environ['CALLS']) if json.loads(line)[:3] == ['xcrun', 'notarytool', 'wait']) - 1
    if failed < int(os.environ.get('NOTARY_WAIT_FAILURES', '0')): sys.exit(1)
    print(json.dumps({'id':'submission', 'status':os.environ['NOTARY_STATUS']}))
if name == 'otool' and os.environ['NOTARY_STATUS'] == 'LocalLibrary': print('/opt/homebrew/lib/libssl.dylib')
if name == 'lipo':
    assert len(args) == 3 and args[0] == '-verify_arch'
    assert args[1] in ['arm64', 'x86_64'] and pathlib.Path(args[2]).is_absolute()
    if os.environ['NOTARY_STATUS'] == 'MissingArchitecture' and args[1] == 'x86_64': sys.exit(1)
if name == 'ditto': pathlib.Path(args[-1]).touch()
`;
    writeFileSync(join(root, 'bin/mock'), mock, { mode: 0o755 });
    for (const name of ['openssl', 'security', 'swift', 'xcodebuild', 'lipo', 'codesign', 'otool', 'ditto', 'xcrun', 'spctl', 'sleep']) symlinkSync('mock', join(root, 'bin', name));
    symlinkSync('bin/mock', join(root, 'nx'));
    const callsPath = join(root, 'calls');
    const result = spawnSync('bash', ['.github/scripts/macos-release.sh'], {
      cwd: root, encoding: 'utf8', env: { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, RUNNER_TEMP: root,
        CALLS: callsPath, NOTARY_STATUS: status, APPLE_CERTIFICATE: 'Y2VydA==', APPLE_CERTIFICATE_PASSWORD: 'fake',
        APPLE_SIGNING_IDENTITY: 'Developer ID Application: Test', APPLE_ID: 'test@example.invalid',
        APPLE_PASSWORD: 'fake', APPLE_TEAM_ID: 'TEST', ...env },
    });
    const calls = readFileSync(callsPath, 'utf8').trim().split('\n').map(JSON.parse);
    return { root, result, calls };
}

for (const status of ['Accepted', 'Invalid', 'LocalLibrary', 'MissingArchitecture']) {
  test(`notarization ${status}: only accepted apps are stapled and packaged, keychain is always cleaned`, t => {
    const { root, result, calls } = release(t, status);
    assert.equal(result.status === 0, status === 'Accepted', result.stdout + result.stderr);
    assert.ok(calls.some(call => call[0] === 'security' && call[1] === 'delete-keychain'));
    assert.ok(calls.some(call => JSON.stringify(call) === JSON.stringify(['security', 'list-keychains', '-d', 'user', '-s', '/tmp/login.keychain'])));
    const stapled = calls.some(call => call[0] === 'xcrun' && call[1] === 'stapler' && call[2] === 'staple');
    assert.equal(stapled, status === 'Accepted');
    assert.equal(existsSync(join(root, 'clients/macos/.build/Distribution/Releash_Preview_0.4.20_universal.zip')), status === 'Accepted');
    const signatures = calls.filter(call => call[0] === 'codesign' && call.includes('--force'));
    assert.equal(signatures.length, ['LocalLibrary', 'MissingArchitecture'].includes(status) ? 0 : 3);
    assert.ok(signatures.every(call => call.includes('runtime') && call.includes('--timestamp')));
    const archiveIndex = calls.findIndex(call => call[0] === 'xcodebuild' && call.includes('archive'));
    if (['LocalLibrary', 'MissingArchitecture'].includes(status)) {
      assert.equal(archiveIndex, -1);
    } else {
      for (const helper of ['releashd', 'releash']) {
        const signatureIndex = calls.findIndex(call => call[0] === 'codesign' && call.at(-1) === join(root, 'clients/macos/.build/Helpers/Release', helper));
        assert.ok(signatureIndex >= 0 && signatureIndex < archiveIndex);
      }
      assert.ok(calls.indexOf(signatures.at(-1)) > archiveIndex);
    }
  });
}

test('notarization wait is retried after transient failures and gives up after five attempts', t => {
  const retried = release(t, 'Accepted', { NOTARY_WAIT_FAILURES: '2' });
  assert.equal(retried.result.status, 0, retried.result.stdout + retried.result.stderr);
  const waits = retried.calls.filter(call => call[0] === 'xcrun' && call[1] === 'notarytool' && call[2] === 'wait');
  assert.equal(waits.length, 3);
  assert.equal(retried.calls.filter(call => call[0] === 'xcrun' && call[1] === 'notarytool' && call[2] === 'submit').length, 1);
  assert.deepEqual(retried.calls.filter(call => call[0] === 'sleep').map(call => call[1]), ['30', '60']);
  assert.ok(retried.calls.some(call => call[0] === 'xcrun' && call[1] === 'stapler' && call[2] === 'staple'));

  const exhausted = release(t, 'Accepted', { NOTARY_WAIT_FAILURES: '5' });
  assert.notEqual(exhausted.result.status, 0);
  assert.equal(exhausted.calls.filter(call => call[0] === 'xcrun' && call[1] === 'notarytool' && call[2] === 'wait').length, 5);
  assert.ok(!exhausted.calls.some(call => call[0] === 'xcrun' && call[1] === 'stapler'));
  assert.ok(exhausted.calls.some(call => call[0] === 'security' && call[1] === 'delete-keychain'));
});
