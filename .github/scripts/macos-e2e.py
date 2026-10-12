import argparse
import ctypes
import fcntl
import io
import json
import os
import plistlib
import re
from pathlib import Path
import subprocess
import tarfile
import tempfile

ROOT = Path(__file__).resolve().parents[2]
LOCK = Path('/tmp') / f'releash-macos-e2e-{os.getuid()}.lock'


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


def test_results(report):
    results = []

    def visit(node):
        if node['nodeType'] == 'Test Case':
            results.append(node)
        for child in node.get('children', []):
            visit(child)

    for node in report['testNodes']:
        visit(node)
    if not results:
        raise ValueError('xcresult contains no test cases')
    return results


def evidence_body(commit, results, root):
    lines = [f'Commit: `{commit}`', '', '| Test | Result |', '|---|---|']
    files = set()
    for result in results:
        name = result.get('nodeIdentifier', result['name'])
        lines.append(f"| `{name}` | {result['result']} |")
        location = result.get('sourceLocation')
        if location:
            path = Path(location['filePath'].removeprefix('file://')).resolve().relative_to(root.resolve())
            if not str(path).startswith('clients/macos/UITests/'):
                raise ValueError(f'Unexpected UI test source: {path}')
        else:
            class_name, method = name.rsplit('/', 1)
            method = method.removesuffix('()')
            matches = [path.relative_to(root) for path in (root / 'clients/macos/UITests').rglob('*.swift')
                       if re.search(r'\bclass\s+' + re.escape(class_name) + r'\b', path.read_text())
                       and re.search(r'\bfunc\s+' + re.escape(method) + r'\s*\(', path.read_text())]
            if len(matches) != 1:
                raise ValueError(f'Cannot identify unique test source: {name}')
            path = matches[0]
        files.add(path)
    for path in sorted(files):
        source = (root / path).read_text()
        fence = '`' * max(3, max((len(part) for part in source.split('\n') if set(part) == {'`'}), default=0) + 1)
        lines.extend(['', f'<details><summary>{path}</summary>', '', f'{fence}swift', source, fence, '', '</details>'])
    return '\n'.join(lines) + '\n'


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
               '-derivedDataPath', '.build/E2EDerivedData', '-disableAutomaticPackageResolution',
               '-parallel-testing-enabled', 'NO', '-resultBundlePath', str(result),
               'CODE_SIGNING_ALLOWED=YES', 'CODE_SIGN_IDENTITY=-', 'test']
    command.extend(f'-only-testing:{test}' for test in tests)
    return monitored_test(command, macos, require_production)


def main(args):
    # All checkouts share this lock because XCUITest needs the foreground application.
    with open(LOCK, 'a') as lock:
        fcntl.flock(lock, fcntl.LOCK_EX)
        if not args.evidence:
            with tempfile.TemporaryDirectory(prefix='releash-e2e-result-') as directory:
                return execute(ROOT, args.test, Path(directory) / 'Tests.xcresult', args.verify_production)
        branch = output(['git', 'symbolic-ref', '--short', 'HEAD'], ROOT)
        pr = json.loads(output(['gh', 'pr', 'view', branch, '--json', 'number'], ROOT))['number']
        commit = output(['git', 'rev-parse', 'HEAD'], ROOT)
        with tempfile.TemporaryDirectory(prefix='releash-evidence-') as directory:
            root = Path(directory) / 'source'
            root.mkdir()
            archive = subprocess.check_output(['git', 'archive', commit], cwd=ROOT)
            with tarfile.open(fileobj=io.BytesIO(archive)) as tar:
                tar.extractall(root, filter='data')
            run(['./nx', 'run-many', '-t', 'generate', 'helpers', '-p', 'macos', '--parallel=2'], root)
            run(['./nx', 'run', 'server:test-fixture-build'], root)
            result = Path(directory) / 'Tests.xcresult'
            status = execute(root, args.test, result, args.verify_production)
            report = json.loads(output(['xcrun', 'xcresulttool', 'get', 'test-results', 'tests',
                                       '--path', str(result), '--compact'], root))
            body = Path(directory) / 'comment.txt'
            results = test_results(report)
            body.write_text(evidence_body(commit, results, root))
            run(['gh', 'pr', 'comment', str(pr), '--body-file', str(body)], ROOT)
            return status or (0 if all(test['result'] == 'Passed' for test in results) else 1)


if __name__ == '__main__':
    parser = argparse.ArgumentParser()
    parser.add_argument('--build-fixture', action='store_true')
    parser.add_argument('--evidence', action='store_true')
    parser.add_argument('--verify-production', action='store_true', help='Require a running production app and server and verify they survive E2E')
    parser.add_argument('--test', action='append', default=[], help='Xcode target/class/method; omit for all regression tests')
    args = parser.parse_args()
    if args.build_fixture:
        build_fixture(ROOT)
    else:
        raise SystemExit(main(args))
