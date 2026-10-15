import json
import importlib.util
from pathlib import Path
import re
import subprocess
import tempfile
import unittest
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[2]
MACOS = ROOT / "clients/macos"
PROJECT = "clients/macos/Releash.xcodeproj"
APP_LOCK = f"{PROJECT}/project.xcworkspace/xcshareddata/swiftpm/Package.resolved"


class MacOSBuildTest(unittest.TestCase):
    def test_helpers_build_requested_architectures_and_profile(self):
        spec = importlib.util.spec_from_file_location("helpers", ROOT / ".github/scripts/macos-helpers.py")
        helpers = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(helpers)
        for configuration, profile, cargo_profile, architectures in [
            ("Debug", "debug", "dev", ["arm64"]),
            ("Release", "release", "release", ["arm64", "x86_64"]),
        ]:
            with self.subTest(configuration=configuration), tempfile.TemporaryDirectory() as directory:
                root = Path(directory)
                target_dir = root / "custom target"
                with patch.object(helpers, "ROOT", root), patch.object(
                    helpers.subprocess, "check_output", return_value=json.dumps({"target_directory": str(target_dir)})
                ), patch.object(helpers.subprocess, "run") as run:
                    helpers.build_helpers(configuration, architectures)
                calls = [call.args[0] for call in run.call_args_list]
                targets = [helpers.TARGETS[arch] for arch in architectures]
                self.assertEqual(len(calls), len(targets) + 2)
                for call, target in zip(calls, targets):
                    self.assertEqual(call, ["cargo", "build", "--locked", "--profile", cargo_profile,
                                           "--target", target, "-p", "releashd", "--bin", "releashd",
                                           "-p", "releash", "--bin", "releash",
                                           *(["--features", "releashd/vendored-openssl"] if configuration == "Release" else [])])
                for call, helper in zip(calls[len(targets):], ["releashd", "releash"]):
                    self.assertEqual(call, ["lipo", "-create", *[
                        str(target_dir / target / profile / helper) for target in targets
                    ], "-output", str(root / "clients/macos/.build/Helpers" / configuration / helper)])
                self.assertTrue(all(call.kwargs["check"] for call in run.call_args_list))

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

    def test_protocol_generation_precedes_source_validation(self):
        project = (MACOS / "project.yml").read_text()
        targets = json.loads((MACOS / "project.json").read_text())["targets"]
        prepare = targets["prepare"]
        self.assertIn("generate", prepare["dependsOn"])
        self.assertIn("helpers", prepare["dependsOn"])
        commands = prepare["options"]["commands"]
        self.assertLess(commands.index("swift run --package-path BuildTools xcodegen generate"),
                        next(i for i, command in enumerate(commands) if command.startswith("xcodebuild ")))
        for target in ["build", "test-unit", "test-integration"]:
            self.assertIn("prepare", targets[target]["dependsOn"])
            self.assertFalse(targets[target]["options"]["forwardAllArgs"])
        self.assertIn("- path: Generated", project)
        self.assertIn("path: Generated/client-descriptor.bin\n        buildPhase: resources", project)
        unit = project.split("  ReleashUnitTests:", 1)[1].split("  ReleashIntegrationTests:", 1)[0]
        self.assertIn("      - Tests/Unit", unit)
        self.assertIn("      - UITests/Fixture.swift", unit)
        self.assertIn("- target: Releash", project)

    def test_helpers_are_copied_and_signed_by_xcode(self):
        project = (MACOS / "project.yml").read_text()
        for helper in ["releashd", "releash"]:
            self.assertIn(f"path: .build/Helpers/$(CONFIGURATION)/{helper}\n        optional: true", project)
        self.assertEqual(project.count("subpath: Contents/Helpers"), 2)
        self.assertEqual(project.count("attributes: [CodeSignOnCopy]"), 2)

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
        self.assertEqual(set(app_versions), {"swift-protobuf", "connect-swift", "swiftterm"})
        for identity, version in tools.items():
            with self.subTest(identity=identity):
                self.assertEqual(tool_pins[identity]["version"], version)
        for identity, version in app_versions.items():
            with self.subTest(identity=identity):
                self.assertEqual(app_pins[identity]["version"], version)
                if identity in tools:
                    self.assertEqual(tools[identity], version)
                    self.assertEqual(app_pins[identity], tool_pins[identity])

    def test_dependency_resolution_updates_both_locks_before_generating_the_project(self):
        target = json.loads((MACOS / "project.json").read_text())["targets"]["resolve-dependencies"]
        self.assertFalse(target["cache"])
        self.assertFalse(target["options"]["parallel"])
        self.assertIn("generate", target["dependsOn"])
        self.assertEqual(target["options"]["cwd"], "clients/macos")
        commands = target["options"]["commands"]
        self.assertEqual(commands[0], "swift package --package-path BuildTools resolve")
        self.assertEqual(commands[1], "swift run --package-path BuildTools xcodegen generate")
        self.assertTrue(commands[2].startswith("xcodebuild -resolvePackageDependencies"))
        self.assertNotIn("-onlyUsePackageVersionsFromResolvedFile", commands[2])
        self.assertNotIn("-disableAutomaticPackageResolution", commands[2])


if __name__ == "__main__":
    unittest.main()
