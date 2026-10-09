import json
from pathlib import Path
import re
import subprocess
import unittest


ROOT = Path(__file__).resolve().parents[2]
MACOS = ROOT / "clients/macos"
PROJECT = "clients/macos/Releash.xcodeproj"
APP_LOCK = f"{PROJECT}/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"


class MacOSBuildTest(unittest.TestCase):
    def test_helper_profiles_expand_with_system_bash(self):
        script = (MACOS / "Scripts/bundle-helpers.sh").read_text()
        setup = script[script.index("profile=debug") : script.index("for arch in")]
        for configuration, expected in [
            ("Debug", "debug\n--profile\ndev\n"),
            ("Release", "release\n--profile\nrelease\n"),
        ]:
            with self.subTest(configuration=configuration):
                result = subprocess.run(
                    ["/bin/bash", "-uc", setup + '\nprintf "%s\\n" "$profile" "${cargo_flags[@]}"'],
                    env={"CONFIGURATION": configuration},
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout, expected)

    def test_only_app_lockfile_is_not_ignored(self):
        for path, ignored in [
            (APP_LOCK, False),
            (f"{PROJECT}/project.pbxproj", True),
            (f"{PROJECT}/project.xcworkspace/contents.xcworkspacedata", True),
            (f"{PROJECT}/project.xcworkspace/xcshareddata/WorkspaceSettings.xcsettings", True),
            (f"{PROJECT}/project.xcworkspace/xcshareddata/swiftpm/configuration/mirrors.json", True),
            (f"{PROJECT}/xcuserdata/user.xcuserdatad/xcschemes/scheme.xcscheme", True),
            ("clients/macos/Other.xcodeproj/project.pbxproj", True),
            ("clients/macos/BuildTools/.build/workspace-state.json", True),
            ("clients/macos/BuildTools/Package.resolved", False),
        ]:
            with self.subTest(path=path):
                result = subprocess.run(
                    ["git", "check-ignore", "--no-index", "-q", path], cwd=ROOT
                )
                self.assertEqual(result.returncode, 0 if ignored else 1)

    def test_tools_and_app_share_exact_dependency_versions(self):
        manifest = (MACOS / "BuildTools/Package.swift").read_text()
        tools = {
            url.rsplit("/", 1)[-1].lower(): version
            for url, version in re.findall(r'\.package\(url: "([^"]+)", exact: "([^"]+)"\)', manifest)
        }
        tool_pins = {
            pin["identity"]: pin["state"]
            for pin in json.loads((MACOS / "BuildTools/Package.resolved").read_text())["pins"]
        }
        app_pins = {
            pin["identity"]: pin["state"]
            for pin in json.loads((ROOT / APP_LOCK).read_text())["pins"]
        }
        app_versions = {
            url.rsplit("/", 1)[-1].lower(): version
            for url, version in re.findall(
                r'url: (\S+)\s+exactVersion: (\S+)', (MACOS / "project.yml").read_text()
            )
        }
        self.assertEqual(set(tools), {"xcodegen", "swift-protobuf", "connect-swift"})
        self.assertEqual(app_versions, {"swift-protobuf": "1.38.1", "connect-swift": "1.0.0"})
        for identity, version in tools.items():
            with self.subTest(identity=identity):
                self.assertEqual(tool_pins[identity]["version"], version)
        for identity, version in app_versions.items():
            with self.subTest(identity=identity):
                self.assertEqual(tools[identity], version)
                self.assertEqual(app_pins[identity], tool_pins[identity])


if __name__ == "__main__":
    unittest.main()
