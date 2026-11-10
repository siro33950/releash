import importlib.util
import json
import plistlib
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

spec = importlib.util.spec_from_file_location('verify_release', Path(__file__).with_name('verify-release.py'))
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


class ReleaseTests(unittest.TestCase):
    def test_配布版_選択元とappと両helperが一致しなければ拒否する(self):
        for component in [None, 'app', 'server', 'client']:
            with self.subTest(component=component), tempfile.TemporaryDirectory() as directory:
                # Given
                root = Path(directory).resolve()
                (root / 'Cargo.toml').write_text('[workspace.package]\nversion = "0.4.21"\n')
                app = root / 'Releash.app'
                (app / 'Contents').mkdir(parents=True)
                (app / 'Contents/Info.plist').write_bytes(plistlib.dumps({
                    'CFBundleVersion': '0.4.22' if component == 'app' else '0.4.21',
                    'CFBundleShortVersionString': '0.4.21', 'CFBundleIdentifier': 'com.releash.macos'}))
                server = Mock(pid=123)
                server.poll.return_value = None
                status = {'running': True, 'server': {'pid': 123, 'release': '0.4.21'}, 'client': {'release': '0.4.21'}}
                if component in ('server', 'client'):
                    status[component]['release'] = '0.4.22'
                result = Mock(returncode=0, stdout=json.dumps(status))
                with patch.object(module.subprocess, 'Popen', return_value=server) as spawn, patch.object(module.subprocess, 'run', return_value=result) as run:
                    # When / Then
                    if component:
                        with self.assertRaises(AssertionError):
                            module.verify(root, app)
                    else:
                        self.assertEqual(module.verify(root, app), '0.4.21')
                    if component != 'app':
                        self.assertEqual(spawn.call_args.args[0][0], str(app / 'Contents/Helpers/releashd'))
                        self.assertEqual(run.call_args.args[0][0], str(app / 'Contents/Helpers/releash'))
                        server.terminate.assert_called_once()
                        server.wait.assert_called_once()


if __name__ == '__main__':
    unittest.main()
