"""Verify package contents and application metadata without installing an app."""
import importlib.util
import json
from pathlib import Path
import plistlib
import tarfile
import tempfile
import unittest
from unittest import mock

SPEC = importlib.util.spec_from_file_location("package_app", Path(__file__).resolve().parents[1] / "packaging/package_app.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class NativePackageTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.base = Path(self.temp.name)
        self.root = self.base / "source"
        self.out = self.base / "dist"
        files = {
            "Cargo.toml": '[package]\nname="screencue"\nversion="0.0.4"\n',
            "LICENSE": "Project license", "docs/INSTALL.md": "Install: [Capture privacy](CAPTURE-PRIVACY.md)",
            "docs/CAPTURE-PRIVACY.md": "Privacy", "assets/OFL.txt": "Font license",
            "docs/DUAL-SYSTEM-PLAN.md": "Decision provider design and sources",
            "docs/CONNECTIONS.md": "Provider settings",
            "docs/CREDENTIAL-STORAGE.md": "OS credential storage",
            "docs/settings-connections.png": "Test settings image",
            "docs/settings-runtime.png": "Test runtime image",
            "docs/THIRD-PARTY-LICENSES.txt": "Overlay notices",
            "conversation/THIRD-PARTY-LICENSES.txt": "Helper notices",
            "conversation/README.md": "Helper usage", "conversation/example.json": "{}",
            "conversation/private.json": '{"secret":"must not ship"}',
            "controller/screencue_controller.py": "# Test controller fixture",
            "controller/conversation_orchestrator.py": "# Test orchestrator fixture",
            "controller/README.md": "Typed Choice provider configuration",
            "controller/CONVERSATION.md": "Orchestrator usage",
            "controller/conversation.example.json": "{}",
            "assets/app-icon-256.png": "Test image fixture", "assets/app-icon.icns": "Test icon fixture",
            "packaging/linux/install.sh": "#!/bin/sh\n", "packaging/linux/uninstall.sh": "#!/bin/sh\n",
            "packaging/linux/screencue.desktop": "[Desktop Entry]\n",
            "overlay": "Test executable fixture", "helper": "Test helper fixture",
        }
        for name, content in files.items():
            path = self.root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_text(content, encoding="utf-8")
        patch = mock.patch.object(PACKAGE, "ROOT", self.root)
        patch.start()
        self.addCleanup(patch.stop)

    def package(self, platform):
        return PACKAGE.package(platform, self.root / "overlay", self.root / "helper", self.out)

    def test_windows_stage_includes_helper_and_notices_but_no_private_config(self):
        result = self.package("windows-x64")
        stage = Path(result["stage"])
        self.assertTrue((stage / "screencue.exe").is_file())
        self.assertTrue((stage / "screencue-listen.exe").is_file())
        self.assertTrue((stage / "licenses/CONVERSATION-THIRD-PARTY-LICENSES.txt").is_file())
        self.assertTrue((stage / "conversation/example.json").is_file())
        self.assertTrue((stage / "controller/conversation_orchestrator.py").is_file())
        self.assertTrue((stage / "controller/README.md").is_file())
        self.assertTrue((stage / "docs/DUAL-SYSTEM-PLAN.md").is_file())
        self.assertTrue((stage / "docs/CONNECTIONS.md").is_file())
        self.assertTrue((stage / "docs/CREDENTIAL-STORAGE.md").is_file())
        self.assertIn("](docs/CAPTURE-PRIVACY.md)", (stage / "README.md").read_text(encoding="utf-8"))
        self.assertIn("](CAPTURE-PRIVACY.md)", (self.root / "docs/INSTALL.md").read_text(encoding="utf-8"))
        self.assertFalse((stage / "conversation/private.json").exists())
        self.assertFalse(json.loads((stage / "package.json").read_text())["signed"])

    def test_missing_helper_notices_fail_packaging(self):
        (self.root / "conversation/THIRD-PARTY-LICENSES.txt").unlink()
        with self.assertRaises(FileNotFoundError):
            self.package("windows-x64")

    def test_preexisting_stage_is_rejected_to_avoid_shipping_stale_files(self):
        self.package("windows-x64")
        with self.assertRaises(FileExistsError):
            self.package("windows-x64")

    def test_linux_archive_contains_executable_installers_and_both_binaries(self):
        result = self.package("linux-x64-x11")
        with tarfile.open(result["artifact"], "r:gz") as archive:
            for name in ["screencue", "screencue-listen", "install.sh", "uninstall.sh"]:
                self.assertTrue(archive.getmember("ScreenCue/" + name).mode & 0o111, name)
            for name in ["controller/README.md", "docs/DUAL-SYSTEM-PLAN.md"]:
                self.assertTrue(archive.getmember("ScreenCue/" + name).isfile(), name)

    def test_macos_bundle_has_audio_permission_purpose_strings_and_sibling_helper(self):
        with mock.patch.object(PACKAGE.subprocess, "run") as run, mock.patch.object(Path, "symlink_to") as symlink:
            result = self.package("macos-apple-silicon")
        app = Path(result["stage"])
        with (app / "Contents/Info.plist").open("rb") as stream:
            info = plistlib.load(stream)
        self.assertEqual(info["CFBundleExecutable"], "screencue")
        self.assertEqual(info["LSMinimumSystemVersion"], "14.2")
        self.assertIn("NSMicrophoneUsageDescription", info)
        self.assertIn("NSAudioCaptureUsageDescription", info)
        self.assertTrue((app / "Contents/MacOS/screencue-listen").is_file())
        self.assertTrue((app / "Contents/Resources/controller/conversation_orchestrator.py").is_file())
        self.assertTrue((app / "Contents/Resources/controller/README.md").is_file())
        self.assertTrue((app / "Contents/Resources/docs/DUAL-SYSTEM-PLAN.md").is_file())
        self.assertIn("](docs/CAPTURE-PRIVACY.md)", (app / "Contents/Resources/README.md").read_text(encoding="utf-8"))
        self.assertFalse((app / "Contents/MacOS/controller").exists())
        self.assertEqual({p.name for p in (app / "Contents/MacOS").iterdir()}, {"screencue", "screencue-listen"})
        symlink.assert_called_once_with("/Applications")
        self.assertEqual(run.call_args_list[-1].args[0][0], "hdiutil")


if __name__ == "__main__":
    unittest.main()
