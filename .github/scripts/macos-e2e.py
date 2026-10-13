import argparse
import ctypes
import fcntl
import json
import os
import plistlib
from pathlib import Path
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
LOCK = Path(tempfile.gettempdir()) / f'releash-macos-e2e-{os.getuid()}.lock'


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
    manifest = root / 'clients/macos/.build/agent-tui-fixture.json'
    manifest.parent.mkdir(parents=True, exist_ok=True)
    manifest.write_text(json.dumps({'executable': executables[0]}))


def production_processes():
    library = ctypes.CDLL('/usr/lib/libproc.dylib')
    library.proc_pidpath.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint32]
    library.proc_pidpath.restype = ctypes.c_int
    processes = {}
    for line in subprocess.check_output(['ps', '-axo', 'pid=,lstart=,stat=,comm='], text=True).splitlines():
        fields = line.split(None, 7)
        pid, started, status, command = int(fields[0]), tuple(fields[1:6]), fields[6], fields[7]
        if 'Z' in status or Path(command).name not in {'Releash', 'releash-desktop', 'releashd'}:
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
        if info['CFBundleIdentifier'] in {'com.releash.macos', 'com.releash.app'}:
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


def execute(root, tests, result, require_production=False):
    macos = root / 'clients/macos'
    run(['swift', 'run', '--package-path', 'BuildTools', 'xcodegen', 'generate'], macos)
    base = ['xcodebuild', '-project', 'Releash.xcodeproj', '-scheme', 'ReleashE2E',
            '-clonedSourcePackagesDirPath', '.build/SourcePackages']
    run([*base, '-resolvePackageDependencies', '-onlyUsePackageVersionsFromResolvedFile'], macos)
    command = [*base, '-configuration', 'Debug', '-destination', 'platform=macOS',
               '-derivedDataPath', '.build/E2EDerivedData', '-disableAutomaticPackageResolution', '-skipPackagePluginValidation',
               '-parallel-testing-enabled', 'NO', '-test-timeouts-enabled', 'YES',
               '-default-test-execution-time-allowance', '60',
               '-maximum-test-execution-time-allowance', '60', '-resultBundlePath', str(result),
               'CODE_SIGNING_ALLOWED=YES', 'test']
    command.extend(f'-only-testing:{test}' for test in tests)
    return monitored_test(command, macos, require_production)


def main(args):
    # All checkouts share this lock because XCUITest needs the foreground application.
    with open(LOCK, 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        with tempfile.TemporaryDirectory(prefix='releash-e2e-result-') as directory:
            return execute(ROOT, args.test, Path(directory) / 'Tests.xcresult', args.verify_production)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--build-fixture', action='store_true')
    parser.add_argument('--verify-production', action='store_true', help='Require a running production app and server and verify they survive E2E')
    parser.add_argument('--test', action='append', default=[], help='Xcode target/class/method; omit for all regression tests')
    args = parser.parse_args()
    if args.build_fixture:
        build_fixture(ROOT)
    else:
        raise SystemExit(main(args))
