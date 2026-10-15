import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { mkdirSync, mkdtempSync, copyFileSync, writeFileSync, readFileSync, rmSync, existsSync, symlinkSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import test from 'node:test';

for (const status of ['Accepted', 'Invalid', 'LocalLibrary', 'MissingArchitecture']) {
  test(`notarization ${status}: only accepted apps are stapled and packaged, keychain is always cleaned`, t => {
    const root = mkdtempSync(join(tmpdir(), 'releash-signing-'));
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
    print(json.dumps({'id':'submission', 'status':os.environ['NOTARY_STATUS']}))
if name == 'otool' and os.environ['NOTARY_STATUS'] == 'LocalLibrary': print('/opt/homebrew/lib/libssl.dylib')
if name == 'lipo':
    assert len(args) == 3 and args[0] == '-verify_arch'
    assert args[1] in ['arm64', 'x86_64'] and pathlib.Path(args[2]).is_absolute()
    if os.environ['NOTARY_STATUS'] == 'MissingArchitecture' and args[1] == 'x86_64': sys.exit(1)
if name == 'ditto': pathlib.Path(args[-1]).touch()
`;
    writeFileSync(join(root, 'bin/mock'), mock, { mode: 0o755 });
    for (const name of ['openssl', 'security', 'swift', 'xcodebuild', 'lipo', 'codesign', 'otool', 'ditto', 'xcrun', 'spctl']) symlinkSync('mock', join(root, 'bin', name));
    symlinkSync('bin/mock', join(root, 'nx'));
    const callsPath = join(root, 'calls');
    const result = spawnSync('bash', ['.github/scripts/macos-release.sh'], {
      cwd: root, encoding: 'utf8', env: { ...process.env, PATH: `${root}/bin:${process.env.PATH}`, RUNNER_TEMP: root,
        CALLS: callsPath, NOTARY_STATUS: status, APPLE_CERTIFICATE: 'Y2VydA==', APPLE_CERTIFICATE_PASSWORD: 'fake',
        APPLE_SIGNING_IDENTITY: 'Developer ID Application: Test', APPLE_ID: 'test@example.invalid',
        APPLE_PASSWORD: 'fake', APPLE_TEAM_ID: 'TEST' },
    });
    assert.equal(result.status === 0, status === 'Accepted', result.stdout + result.stderr);
    const calls = readFileSync(callsPath, 'utf8').trim().split('\n').map(JSON.parse);
    assert.ok(calls.some(call => call[0] === 'security' && call[1] === 'delete-keychain'));
    assert.ok(calls.some(call => JSON.stringify(call) === JSON.stringify(['security', 'list-keychains', '-d', 'user', '-s', '/tmp/login.keychain'])));
    const stapled = calls.some(call => call[0] === 'xcrun' && call[1] === 'stapler' && call[2] === 'staple');
    assert.equal(stapled, status === 'Accepted');
    assert.equal(existsSync(join(root, 'clients/macos/.build/Distribution/Releash_Preview_0.4.20_universal.zip')), status === 'Accepted');
    const signatures = calls.filter(call => call[0] === 'codesign' && call.includes('--force'));
    assert.equal(signatures.length, ['LocalLibrary', 'MissingArchitecture'].includes(status) ? 0 : 3);
    assert.ok(signatures.every(call => call.includes('runtime') && call.includes('--timestamp')));
  });
}
