import json
import os
import plistlib
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path


def verify(root, app):
    version = tomllib.loads((root / 'Cargo.toml').read_text())['workspace']['package']['version']
    with (app / 'Contents/Info.plist').open('rb') as file:
        info = plistlib.load(file)
    assert info['CFBundleVersion'] == version, 'Build/Cargo version mismatch'
    assert info['CFBundleShortVersionString'] == version, 'Swift/Cargo version mismatch'
    assert json.loads((root / 'clients/desktop/package.json').read_text())['version'] == version, 'Tauri/Cargo version mismatch'
    assert info['CFBundleIdentifier'] == 'com.releash.macos', 'Unexpected bundle ID'
    helpers = app.resolve() / 'Contents/Helpers'
    with tempfile.TemporaryDirectory(prefix='releash-release-check-') as directory:
        env = dict(os.environ, HOME=directory, XDG_CONFIG_HOME=directory,
                   CODEX_HOME=str(Path(directory) / 'codex'),
                   CLAUDE_CONFIG_DIR=str(Path(directory) / 'claude'),
                   RELEASH_WORKFLOWS_DIR=str(Path(directory) / 'workflows'))
        env.pop('RELEASH_DATA_DIR', None)
        data = Path(directory) / 'data'
        with (Path(directory) / 'server.log').open('w') as log:
            server = subprocess.Popen([str(helpers / 'releashd'), '--data-dir', str(data)],
                                      env=env, cwd=directory, stdout=log, stderr=log)
            try:
                deadline = time.monotonic() + 30
                while time.monotonic() < deadline:
                    if server.poll() is not None:
                        raise RuntimeError('Bundled server exited before verification')
                    result = subprocess.run([str(helpers / 'releash'), '--data-dir', str(data), 'status', '--json'],
                                            env=env, cwd=directory, capture_output=True, text=True, timeout=10)
                    if result.returncode == 0:
                        status = json.loads(result.stdout)
                        if status['running']:
                            assert status['server']['pid'] == server.pid, 'Unexpected server'
                            assert status['server']['release'] == version, 'Bundled server version mismatch'
                            assert status['client']['release'] == version, 'Bundled CLI version mismatch'
                            return version
                    time.sleep(0.1)
                raise RuntimeError('Bundled server did not become ready')
            finally:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait()


if __name__ == '__main__':
    print(verify(*map(Path, sys.argv[1:])))
