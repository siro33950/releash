import importlib.util
import plistlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location('macos_e2e', Path(__file__).with_name('macos-e2e.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ProductionTests(unittest.TestCase):
    def test_本番監視_新旧の実行名を両方監視する(self):
        # Given
        with tempfile.TemporaryDirectory() as directory:
            commands = []
            for index, name in enumerate(['Releash', 'Releash Preview'], start=10):
                bundle = Path(directory) / f'{name}.app'
                (bundle / 'Contents/MacOS').mkdir(parents=True)
                (bundle / 'Contents/Info.plist').write_bytes(plistlib.dumps({'CFBundleIdentifier': 'com.releash.macos'}))
                commands.append(str(bundle / 'Contents/MacOS' / name))
            class Library:
                pass
            library = Library()
            def pidpath(pid, buffer, _size):
                buffer.value = commands[pid - 10].encode()
                return len(buffer.value)
            library.proc_pidpath = pidpath
            output = '\n'.join(f'{i} Fri Nov 6 12:00:00 2026 S {command}' for i, command in enumerate(commands, 10))
            # When
            with patch.object(module.ctypes, 'CDLL', return_value=library), patch.object(module.subprocess, 'check_output', return_value=output):
                processes = module.production_processes()
            # Then
            self.assertEqual({pid for pid, _ in processes}, {10, 11})
            self.assertEqual(set(processes.values()), {'app'})


if __name__ == '__main__':
    unittest.main()
