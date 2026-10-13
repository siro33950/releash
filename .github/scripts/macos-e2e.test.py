import argparse
import importlib.util
import threading
import json
from pathlib import Path
import subprocess
import tempfile
import unittest
from unittest.mock import MagicMock, patch

spec = importlib.util.spec_from_file_location('e2e', Path(__file__).with_name('macos-e2e.py'))
e2e = importlib.util.module_from_spec(spec)
spec.loader.exec_module(e2e)


class E2ETests(unittest.TestCase):
    def setUp(self):
        directory = tempfile.TemporaryDirectory(prefix='e2e-unit-')
        self.addCleanup(directory.cleanup)
        patcher = patch.object(e2e, 'LOCK', Path(directory.name) / 'e2e.lock')
        patcher.start()
        self.addCleanup(patcher.stop)

    def test_fixture_manifest_uses_cargo_artifact_in_custom_target_directory(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory) / 'source'
            stale = root / 'target/debug/releash-agent-tui-fixture'
            stale.parent.mkdir(parents=True)
            stale.write_text('stale binary')
            executable = Path(directory) / 'custom-target/debug/releash-agent-tui-fixture'
            artifacts = [
                {'reason': 'compiler-artifact', 'target': {'name': 'releashd'}, 'executable': None},
                {'reason': 'compiler-artifact', 'target': {'name': 'releash-agent-tui-fixture'}, 'executable': str(executable)},
                {'reason': 'build-finished', 'success': True},
            ]
            with patch.object(e2e.subprocess, 'run', return_value=subprocess.CompletedProcess([], 0, '\n'.join(map(json.dumps, artifacts)))) as run:
                e2e.build_fixture(root)
            manifest = json.loads((root / 'clients/macos/.build/agent-tui-fixture.json').read_text())
            self.assertEqual(manifest, {'executable': str(executable)})
            self.assertNotEqual(manifest['executable'], str(stale))
            self.assertTrue(run.call_args.kwargs['check'])
            self.assertIn('--message-format=json-render-diagnostics', run.call_args.args[0])

    def test_fixture_build_does_not_reuse_manifest_after_cargo_failure(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            with patch.object(e2e.subprocess, 'run', side_effect=subprocess.CalledProcessError(1, ['cargo'])):
                with self.assertRaises(subprocess.CalledProcessError):
                    e2e.build_fixture(root)
            self.assertFalse((root / 'clients/macos/.build/agent-tui-fixture.json').exists())

    def test_second_execution_waits_for_first_execution(self):
        first_entered = threading.Event()
        second_entered = threading.Event()
        release_first = threading.Event()
        calls = []
        outcomes = []

        def execute(root, tests, result, require_production):
            calls.append(tests)
            if tests == ['first']:
                first_entered.set()
                self.assertTrue(release_first.wait(5))
            else:
                second_entered.set()
            return 0

        with patch.object(e2e, 'execute', side_effect=execute):
            first = threading.Thread(target=lambda: outcomes.append(e2e.main(argparse.Namespace(verify_production=False, test=['first']))))
            second = threading.Thread(target=lambda: outcomes.append(e2e.main(argparse.Namespace(verify_production=False, test=['second']))))
            first.start()
            try:
                self.assertTrue(first_entered.wait(5))
                second.start()
                self.assertFalse(second_entered.wait(0.05))
            finally:
                release_first.set()
                first.join(5)
                if second.ident is not None:
                    second.join(5)
            self.assertFalse(first.is_alive())
            self.assertFalse(second.is_alive())
        self.assertEqual(calls, [['first'], ['second']])
        self.assertEqual(outcomes, [0, 0])

    def test_selected_tests_disable_parallel_execution_and_preserve_failure(self):
        with patch.object(e2e, 'run'), patch.object(e2e, 'monitored_test', return_value=65) as run:
            self.assertEqual(e2e.execute(Path('/source'), ['ReleashUITests/WorkbenchTests/testExample'], Path('/result')), 65)
            command = run.call_args.args[0]
            self.assertIn('-only-testing:ReleashUITests/WorkbenchTests/testExample', command)
            self.assertEqual(command[command.index('-parallel-testing-enabled') + 1], 'NO')
            self.assertEqual(command[command.index('-test-timeouts-enabled') + 1], 'YES')
            self.assertEqual(command[command.index('-maximum-test-execution-time-allowance') + 1], '300')
            self.assertIn('CODE_SIGNING_ALLOWED=YES', command)
            self.assertFalse(any(argument.startswith('CODE_SIGN_IDENTITY=') for argument in command))

    def test_production_monitor_requires_both_app_and_server(self):
        with patch.object(e2e, 'production_processes', return_value={(10, ('start',)): 'app'}), patch.object(e2e.subprocess, 'Popen') as popen:
            with self.assertRaises(ValueError):
                e2e.monitored_test(['xcodebuild'], Path('/source'), True)
            popen.assert_not_called()

    def test_e2e_can_run_without_production_unless_verification_is_required(self):
        with patch.object(e2e, 'production_processes', return_value={}), patch.object(e2e.subprocess, 'Popen') as popen:
            popen.return_value.__enter__.return_value.wait.return_value = 0
            self.assertEqual(e2e.monitored_test(['xcodebuild'], Path('/source'), False), 0)

    def test_process_snapshot_identifies_production_bundle_and_excludes_development_and_zombies(self):
        with tempfile.TemporaryDirectory() as directory:
            paths = {}
            for name, identifier, app_pid, daemon_pid in [
                ('Production', 'com.releash.macos', 10, 20),
                ('Desktop', 'com.releash.app', 30, 40),
                ('Development', 'com.releash.macos.dev', 50, 60),
            ]:
                bundle = Path(directory) / f'{name}.app/Contents'
                bundle.mkdir(parents=True)
                with (bundle / 'Info.plist').open('wb') as file:
                    e2e.plistlib.dump({'CFBundleIdentifier': identifier}, file)
                paths[app_pid] = str(bundle / 'MacOS/Releash')
                paths[daemon_pid] = str(bundle / 'Helpers/releashd')
            rows = '\n'.join(f'{pid} Mon Oct 12 10:00:00 2026 S {"Releash" if pid % 20 == 10 else "releashd"}' for pid in paths)
            rows += '\n70 Mon Oct 12 10:00:00 2026 Z releashd'
            library = MagicMock()

            def pidpath(pid, buffer, size):
                buffer.value = paths[pid].encode()
                return len(buffer.value)

            library.proc_pidpath.side_effect = pidpath
            with patch.object(e2e.ctypes, 'CDLL', return_value=library), patch.object(e2e.subprocess, 'check_output', return_value=rows):
                processes = e2e.production_processes()
            self.assertEqual({pid: role for (pid, _), role in processes.items()}, {10: 'app', 20: 'server', 30: 'app', 40: 'server'})

    def test_production_monitor_checks_during_execution_and_after_teardown(self):
        before = {(10, ('start',)): 'app', (20, ('start',)): 'server'}
        process = MagicMock()
        process.wait.side_effect = [subprocess.TimeoutExpired('xcodebuild', 0.5), 65]
        with patch.object(e2e, 'production_processes', side_effect=[before] * 4) as snapshot, patch.object(e2e.subprocess, 'Popen') as popen:
            popen.return_value.__enter__.return_value = process
            self.assertEqual(e2e.monitored_test(['xcodebuild'], Path('/source'), True), 65)
            self.assertEqual(snapshot.call_count, 4)
            process.terminate.assert_not_called()

    def test_production_monitor_rejects_reused_pid_during_execution(self):
        before = {(10, ('start',)): 'app', (20, ('start',)): 'server'}
        restarted = {(10, ('start',)): 'app', (20, ('later',)): 'server'}
        with patch.object(e2e, 'production_processes', side_effect=[before, restarted]), patch.object(e2e.subprocess, 'Popen') as popen:
            with self.assertRaisesRegex(RuntimeError, 'during E2E'):
                e2e.monitored_test(['xcodebuild'], Path('/source'), True)
            popen.return_value.__enter__.return_value.terminate.assert_called_once()

    def test_production_monitor_rejects_exit_after_teardown(self):
        before = {(10, ('start',)): 'app', (20, ('start',)): 'server'}
        with patch.object(e2e, 'production_processes', side_effect=[before, before, {}]), patch.object(e2e.subprocess, 'Popen') as popen:
            popen.return_value.__enter__.return_value.wait.return_value = 0
            with self.assertRaisesRegex(RuntimeError, 'after E2E teardown'):
                e2e.monitored_test(['xcodebuild'], Path('/source'), True)


if __name__ == '__main__':
    unittest.main()
