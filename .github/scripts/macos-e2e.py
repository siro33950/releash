import argparse
import ctypes
import fcntl
import importlib.util
import json
import os
import plistlib
from pathlib import Path
import re
import shutil
import subprocess
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[2]
LOCK = Path(tempfile.gettempdir()) / f'releash-macos-e2e-{os.getuid()}.lock'
_spec = importlib.util.spec_from_file_location('macos_helpers', Path(__file__).with_name('macos-helpers.py'))
helpers = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(helpers)


def run(command, root, **kwargs):
    return subprocess.run(command, cwd=root, check=True, **kwargs)


def output(command, root):
    return subprocess.check_output(command, cwd=root, text=True).strip()


def build_fixture(root):
    build = subprocess.run([
        'cargo', 'build', '--locked', '-p', 'releashd', '--features', 'test-support',
        '--bin', 'releash-agent-tui-fixture', '--message-format=json-render-diagnostics',
    ], cwd=root, check=True, stdout=subprocess.PIPE, text=True)
    artifacts = [json.loads(line) for line in build.stdout.splitlines()]
    executables = [artifact['executable'] for artifact in artifacts
                   if artifact.get('reason') == 'compiler-artifact'
                   and artifact['target']['name'] == 'releash-agent-tui-fixture'
                   and artifact.get('executable')]
    if len(executables) != 1:
        raise ValueError('Cargo did not report one TUI fixture executable')
    helpers.sign_development([executables[0]])
    manifest = root / 'clients/macos/.build/agent-tui-fixture.json'
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(json.dumps({'executable': executables[0]}))


def build_updated_cli(root):
    manifest = (root / 'Cargo.toml').read_text()
    version = tomllib.loads(manifest)['workspace']['package']['version']
    major, minor, patch = version.split('.')
    updated = f'{major}.{minor}.{int(patch) + 1}'
    build = root / 'clients/macos/.build'
    with tempfile.TemporaryDirectory(prefix='releash-update-cli-') as directory:
        workspace = Path(directory)
        shutil.copytree(root / 'proto', workspace / 'proto')
        shutil.copytree(root / 'clients/cli', workspace / 'clients/cli')
        shutil.copy(root / 'Cargo.lock', workspace)
        shutil.copy(root / 'rust-toolchain.toml', workspace)
        manifest = re.sub(r'^members = .*$', 'members = ["clients/cli"]', manifest, flags=re.M)
        (workspace / 'Cargo.toml').write_text(manifest.replace(f'version = "{version}"', f'version = "{updated}"', 1))
        target = build / 'UpdateCLI'
        result = subprocess.run([
            'cargo', 'build', '-p', 'releash', '--bin', 'releash', '--target-dir', str(target),
            '--message-format=json-render-diagnostics',
        ], cwd=workspace, check=True, stdout=subprocess.PIPE, text=True)
        artifacts = [json.loads(line) for line in result.stdout.splitlines()]
        executables = [artifact['executable'] for artifact in artifacts
                       if artifact.get('reason') == 'compiler-artifact'
                       and artifact['target']['name'] == 'releash'
                       and artifact.get('executable')]
        if len(executables) != 1:
            raise ValueError('Cargo did not report one CLI executable')
    output = build / 'Helpers/Update/releash'
    output.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(executables[0], output)
    helpers.sign_development([output])


def production_processes():
    library = ctypes.CDLL('/usr/lib/libproc.dylib')
    library.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    library.proc_pidpath.restype = ctypes.c_int
    processes = {}
    for line in subprocess.check_output(['ps', '-axo', 'pid=,lstart=,stat=,comm='], text=True).splitlines():
        fields = line.split(None, 7)
        pid, started, status, command = int(fields[0]), tuple(fields[1:6]), fields[6], fields[7]
        if 'Z' in status or Path(command).name not in {'Releash', 'Releash Preview', 'releashd'}:
            continue
        buffer = ctypes.create_string_buffer(4096)
        if library.proc_pidpath(pid, buffer, len(buffer)) <= 0:
            continue
        executable = Path(os.fsdecode(buffer.value))
        bundle = next((parent for parent in executable.parents if parent.suffix == '.app'), None)
        if bundle is None:
            continue
        with (bundle / 'Contents/Info.plist').open('rb') as file:
            info = plistlib.load(file)
        if info['CFBundleIdentifier'] == 'com.releash.macos':
            role = 'server' if executable.name == 'releashd' else 'app'
            processes[(pid, started)] = role
    return processes


def monitored_test(command, macos, require_production):
    before = production_processes()
    if require_production and set(before.values()) != {'app', 'server'}:
        raise ValueError('Production Releash app and server must already be running')
    if before:
        print(f'Production processes before E2E: {before}', flush=True)
    with subprocess.Popen(command, cwd=macos) as process:
        while True:
            current = production_processes()
            if not before.keys() <= current.keys():
                process.terminate()
                raise RuntimeError('Production Releash process exited or restarted during E2E')
            try:
                status = process.wait(timeout=0.5)
                break
            except subprocess.TimeoutExpired:
                pass
    if not before.keys() <= production_processes().keys():
        raise RuntimeError('Production Releash process exited or restarted after E2E teardown')
    if before:
        print(f'Production processes preserved after E2E teardown: {before}', flush=True)
    return status


def execute(root, tests, result, require_production=False, retry=False):
    macos = root / 'clients/macos'
    run(['swift', 'run', '--package-path', 'BuildTools', 'xcodegen', 'generate'], macos)
    base = ['xcodebuild', '-project', 'Releash.xcodeproj', '-scheme', 'ReleashE2E',
            '-clonedSourcePackagesDirPath', '.build/SourcePackages']
    run([*base, '-resolvePackageDependencies', '-onlyUsePackageVersionsFromResolvedFile'], macos)
    command = [*base, '-configuration', 'Debug', '-destination', 'platform=macOS',
               '-derivedDataPath', '.build/E2EDerivedData', '-disableAutomaticPackageResolution', '-skipPackagePluginValidation',
               '-parallel-testing-enabled', 'NO', '-test-timeouts-enabled', 'YES',
               '-default-test-execution-time-allowance', '60',
               '-maximum-test-execution-time-allowance', '300', '-resultBundlePath', str(result),
               'CODE_SIGNING_ALLOWED=YES', 'test']
    if retry:
        command.extend(['-retry-tests-on-failure', '-test-iterations', '2',
                        '-test-repetition-relaunch-enabled', 'YES'])
    command.extend(f'-only-testing:{test}' for test in tests)
    return monitored_test(command, macos, require_production)


def main(args):
    # All checkouts share this lock because XCUITest needs the foreground application.
    with open(LOCK, 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if args.result_bundle:
            result = args.result_bundle.resolve()
            result.parent.mkdir(parents=True, exist_ok=True)
            return execute(ROOT, args.test, result, args.verify_production, args.retry)
        with tempfile.TemporaryDirectory(prefix='releash-e2e-result-') as directory:
            return execute(ROOT, args.test, Path(directory) / 'Tests.xcresult', args.verify_production, args.retry)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--result-bundle', type=Path, help='Preserve the xcresult at this path')
    parser.add_argument('--retry', action='store_true', help='Retry failed tests once')
    parser.add_argument('--build-fixture', action='store_true')
    parser.add_argument('--verify-production', action='store_true', help='Require a running production app and server and verify they survive E2E')
    parser.add_argument('--test', action='append', default=[], help='Xcode target/class/method; omit for all regression tests')
    args = parser.parse_args()
    if args.build_fixture:
        build_fixture(ROOT)
        build_updated_cli(ROOT)
    else:
        raise SystemExit(main(args))
