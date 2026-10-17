"""Offline integration tests; never write to real client profiles or launch AI clients."""
from __future__ import annotations

import argparse
import copy
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import stat
import sys
import tempfile
import tomllib
import unittest
from unittest import mock
import zipfile

REPO = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("screencue_integrate", REPO / "scripts/integrate.py")
integrate = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(integrate)
PACKAGE_SPEC = importlib.util.spec_from_file_location("screencue_package", REPO / "scripts/package_integrations.py")
packaging = importlib.util.module_from_spec(PACKAGE_SPEC)
with mock.patch.dict(sys.modules, {"integrate": integrate}):
    PACKAGE_SPEC.loader.exec_module(packaging)


class IntegrationTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="screencue-integrations-")
        self.addCleanup(self.temporary.cleanup)
        self.base = Path(self.temporary.name).resolve()
        self.root = self.base / "project with spaces"
        self.home = self.base / "isolated home"
        self.exe = self.base / "native tools" / "screencue.exe"
        self.exe.parent.mkdir(parents=True)
        # A packaging fixture, not a runnable executable. No test invokes it.
        self.exe.write_bytes(b"ScreenCue packaging fixture\x00\xff")

    def args(self, **overrides):
        values = dict(client="all", executable=str(self.exe), scope="project",
                      root=str(self.root), home=str(self.home), dry_run=False)
        values.update(overrides)
        return argparse.Namespace(**values)

    def save(self, path, content):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(content, encoding="utf-8")

    def configure(self, **overrides):
        with mock.patch.object(integrate, "native_platform", return_value="windows-x64"):
            return integrate.configure(self.args(**overrides))

    def test_codex_merge_preserves_other_servers_settings_and_comments(self):
        original = '''# User preferences stay intact
model = "example-model"
[features]
experimental = false
[mcp_servers.other]
command = "/some/other/server"
args = ["--special"]
["mcp_servers".'screencue']
url = "https://old.example/mcp"
bearer_token_env_var = "OLD_TOKEN"
http_headers = { "X-Test" = "old" }
env_http_headers = { "X-Token" = "OLD_TOKEN" }
enabled = false
tool_timeout_sec = 45
enabled_tools = ["screen_hint", "screen_clear"]
[mcp_servers.screencue.env]
DISPLAY = ":1"
[profiles.local]
model = "another-model"
'''
        updated = integrate.merge_codex(original, self.exe)
        before, after = tomllib.loads(original), tomllib.loads(updated)
        expected = copy.deepcopy(before)
        entry = expected["mcp_servers"]["screencue"]
        for key in ("url", "http_headers", "env_http_headers", "bearer_token_env_var"):
            entry.pop(key)
        entry.update(command=str(self.exe), args=["--mcp"], enabled=True)
        self.assertEqual(after, expected)
        self.assertIn("# User preferences stay intact", updated)
        self.assertEqual(integrate.merge_codex(updated, self.exe), updated)

    def test_codex_rejects_ambiguous_inline_or_malformed_tables(self):
        for original in (
            'mcp_servers = { screencue = { command = "old" } }',
            'mcp_servers = "not a table"',
            '[mcp_servers]\nscreencue = false',
            '[mcp_servers.screencue\ncommand = "old"',
        ):
            with self.subTest(original=original), self.assertRaises(ValueError):
                integrate.merge_codex(original, self.exe)

    def test_codex_multiline_section_lookalike_is_never_silently_changed(self):
        original = 'notes = """\n[mcp_servers.screencue]\nDo not remove this literal text.\n"""\n'
        try:
            updated = integrate.merge_codex(original, self.exe)
        except ValueError:
            return
        self.assertEqual(tomllib.loads(updated)["notes"], tomllib.loads(original)["notes"])

    def test_json_merge_keeps_unrelated_settings_and_server_permissions(self):
        for client in ("claude-code", "gemini", "cursor", "claude-desktop"):
            with self.subTest(client=client):
                scope = "user" if client == "claude-desktop" else "project"
                destination = integrate.config_path(client, scope, self.root, self.home, "windows-x64")
                old_entry = dict(url="https://old.example/mcp", httpUrl="https://old.example",
                                 type="http", headers={"X-Test": "old"}, env={"DISPLAY": ":1"},
                                 timeout=30000, disabledTools=["screen_clear"])
                previous = {"theme": "dark", "mcpServers": {
                    "unrelated": {"command": "/another/server"}, "screencue": old_entry}}
                self.save(destination, json.dumps(previous))
                plan, skipped = integrate.plan_configuration((client,), self.exe, scope, self.root, self.home, "windows-x64")
                updated = json.loads(plan[destination])
                self.assertFalse(skipped)
                self.assertEqual(updated["theme"], "dark")
                self.assertEqual(updated["mcpServers"]["unrelated"], previous["mcpServers"]["unrelated"])
                entry = updated["mcpServers"]["screencue"]
                self.assertEqual(entry["command"], str(self.exe))
                self.assertEqual(entry["args"], ["--mcp"])
                self.assertEqual(entry["env"], {"DISPLAY": ":1"})
                self.assertEqual(entry["disabledTools"], ["screen_clear"])
                self.assertEqual(entry["timeout"], 30000)
                for key in ("url", "httpUrl", "headers"):
                    self.assertNotIn(key, entry)
                self.assertEqual(entry.get("type"), "stdio" if client == "claude-code" else None)

    def test_all_client_paths_skills_and_cursor_project_rule(self):
        plan, skipped = integrate.plan_configuration(integrate.CLIENTS, self.exe, "project", self.root, self.home, "windows-x64")
        expected = (".codex/config.toml", ".mcp.json", ".gemini/settings.json", ".cursor/mcp.json",
                    ".agents/skills/screencue/SKILL.md", ".claude/skills/screencue/SKILL.md",
                    ".gemini/skills/screencue/SKILL.md", ".cursor/skills/screencue/SKILL.md",
                    ".cursor/rules/screencue.mdc")
        for relative in expected:
            self.assertIn(self.root / relative, plan)
        self.assertEqual(len(skipped), 1)
        self.assertIn("claude-desktop", skipped[0])
        self.assertNotIn(self.home / ".claude/skills/screencue/SKILL.md", plan)
        self.assertEqual(integrate.config_path("claude-desktop", "user", self.root, self.home, "macos-intel"),
                         self.home / "Library/Application Support/Claude/claude_desktop_config.json")
        self.assertIsNone(integrate.config_path("claude-desktop", "user", self.root, self.home, "linux-x64-x11"))

    def test_home_overrides_are_respected_and_isolated_home_does_not_leak(self):
        overrides = {"CODEX_HOME": str(self.base / "custom codex"),
                     "CLAUDE_CONFIG_DIR": str(self.base / "custom claude"),
                     "APPDATA": str(self.base / "custom appdata")}
        with mock.patch.object(integrate.Path, "home", return_value=self.home), mock.patch.dict(os.environ, overrides):
            plan, _ = integrate.plan_configuration(integrate.CLIENTS, self.exe, "user", self.root, self.home, "windows-x64")
            for path in (Path(overrides["CODEX_HOME"]) / "config.toml",
                         Path(overrides["CLAUDE_CONFIG_DIR"]) / ".claude.json",
                         Path(overrides["CLAUDE_CONFIG_DIR"]) / "skills/screencue/SKILL.md",
                         Path(overrides["APPDATA"]) / "Claude/claude_desktop_config.json",
                         self.home / ".agents/skills/screencue/SKILL.md"):
                self.assertIn(path, plan)
            isolated = self.base / "another home"
            plan, _ = integrate.plan_configuration(integrate.CLIENTS, self.exe, "user", self.root, isolated, "windows-x64")
            self.assertTrue(all(path.is_relative_to(isolated) for path in plan))

    def test_dry_run_writes_nothing(self):
        result = self.configure(dry_run=True)
        self.assertTrue(result["files"])
        self.assertTrue(all(item["status"] == "planned" for item in result["files"]))
        self.assertFalse(self.root.exists())
        self.assertFalse(self.home.exists())

    def test_configure_is_idempotent_and_backup_keeps_exact_original(self):
        destination = self.root / ".cursor/mcp.json"
        original = '{"theme":"dark","mcpServers":{"other":{"command":"unchanged"}}}\n'
        self.save(destination, original)
        first = self.configure(client="cursor")
        backups = list(destination.parent.glob("mcp.json.screencue-backup-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_text(encoding="utf-8"), original)
        self.assertTrue(all(item["status"] == "written" for item in first["files"]))
        second = self.configure(client="cursor")
        self.assertTrue(all(item["status"] == "unchanged" for item in second["files"]))
        self.assertEqual(list(destination.parent.glob("mcp.json.screencue-backup-*")), backups)
        self.assertEqual(list(destination.parent.glob(".screencue-*")), [])

    def test_malformed_last_client_settings_fail_before_any_write(self):
        for malformed in ('{broken', '[]', '{"mcpServers":[]}', '{"mcpServers":{"screencue":false}}'):
            with self.subTest(malformed=malformed):
                destination = self.root / ".cursor/mcp.json"
                self.save(destination, malformed)
                with self.assertRaises(ValueError):
                    self.configure()
                self.assertFalse((self.root / ".codex").exists())
                self.assertFalse((self.root / ".mcp.json").exists())
                self.assertFalse((self.root / ".gemini").exists())
                self.assertEqual(destination.read_text(encoding="utf-8"), malformed)
                self.assertEqual(list(destination.parent.glob("*.screencue-backup-*")), [])

    def test_invalid_skill_destination_is_preflighted_before_configuration_write(self):
        (self.root / ".cursor/skills/screencue/SKILL.md").mkdir(parents=True)
        with self.assertRaises(ValueError):
            self.configure(client="cursor")
        self.assertFalse((self.root / ".cursor/mcp.json").exists())

    def test_native_platform_selects_mac_architecture(self):
        for system, machine, expected in (("win32", "AMD64", "windows-x64"),
                                           ("darwin", "arm64", "macos-apple-silicon"),
                                           ("darwin", "x86_64", "macos-intel"),
                                           ("linux", "x86_64", "linux-x64-x11")):
            with self.subTest(system=system, machine=machine), mock.patch.object(integrate.sys, "platform", system), mock.patch("platform.machine", return_value=machine):
                self.assertEqual(integrate.native_platform(), expected)

    def test_doctor_rejects_broken_or_disabled_entries_without_launching_binary(self):
        valid = {"command": str(self.exe), "args": ["--mcp"]}
        malformed_servers = ([], "invalid", {"screencue": []}, {"screencue": True},
                             {"screencue": {"command": 42, "args": ["--mcp"]}},
                             {"screencue": {**valid, "args": ["show"]}})
        invalid_entries = ({**valid, "enabled": False}, {**valid, "disabled": True},
                           {**valid, "url": "https://old.example/mcp"},
                           {**valid, "httpUrl": "https://old.example/mcp"},
                           {**valid, "type": "http"}, {**valid, "type": "sse"})
        for client, key in (("cursor", "mcpServers"), ("codex", "mcp_servers")):
            destination = integrate.config_path(client, "project", self.root, self.home, "windows-x64")
            fixtures = list(malformed_servers) + [{"screencue": entry} for entry in invalid_entries]
            for servers in fixtures:
                with self.subTest(client=client, servers=servers):
                    if client == "codex":
                        content = f"{key} = {integrate.toml_value(servers)}\n"
                    else:
                        content = json.dumps({key: servers})
                    self.save(destination, content)
                    with mock.patch.object(integrate, "probe") as probe, self.assertRaises(ValueError):
                        integrate.doctor(self.args(client=client))
                    probe.assert_not_called()

    def test_doctor_checks_the_configured_executable_and_labels_scope(self):
        self.configure(client="cursor")
        healthy = {"server": {"name": "screencue", "version": "0.2.0"},
                   "tools": sorted(integrate.TOOLS), "visible": False}
        with mock.patch.object(integrate, "probe", return_value=healthy) as probe:
            report = integrate.doctor(self.args(client="cursor"))
        probe.assert_called_once_with(self.exe)
        self.assertEqual(report["checks"][0]["status"], "MCP protocol passed")
        self.assertIn("not the AI client", report["note"])

    def test_aggregate_package_preserves_marketplace_desktop_installers_and_modes(self):
        for platform in ("windows-x64", "macos-apple-silicon", "linux-x64-x11"):
            with self.subTest(platform=platform):
                source = self.base / ("aggregate-input-" + platform)
                integrate.bundle(self.args(platform=platform, output=str(source)))
                output = self.base / ("integrations-" + platform + ".zip")
                self.assertEqual(packaging.package(source, output), output)
                with zipfile.ZipFile(output) as archive:
                    self.assertIsNone(archive.testzip())
                    names = archive.namelist()
                    self.assertEqual(len(names), len(set(names)))
                    for required in ("README.md", "docs/INTEGRATION.md", "docs/VERIFICATION.md",
                                     ".agents/plugins/marketplace.json", "codex/plugin.json",
                                     "claude-code/.claude-plugin/plugin.json", "cursor/.cursor-plugin/plugin.json",
                                     "gemini/gemini-extension.json"):
                        self.assertIn(required, names)
                    marketplace = json.loads(archive.read(".agents/plugins/marketplace.json"))
                    plugin_path = marketplace["plugins"][0]["source"]["path"].removeprefix("./")
                    self.assertIn(plugin_path + "/plugin.json", names)
                    self.assertFalse(any(name.startswith("claude-desktop/") for name in names))
                    nested_zips = {name for name in names if name.endswith(".zip")}
                    desktop = not platform.startswith("linux")
                    self.assertEqual(nested_zips, {"screencue-claude-desktop-skill.zip"} if desktop else set())
                    mcpbs = {name for name in names if name.endswith(".mcpb")}
                    self.assertEqual(mcpbs, {f"screencue-claude-desktop-{platform}.mcpb"} if desktop else set())
                    binary = "screencue.exe" if platform.startswith("windows") else "screencue"
                    for client in ("codex", "claude-code", "gemini", "cursor"):
                        info = archive.getinfo(client + "/bin/" + binary)
                        self.assertEqual(info.create_system, 3)
                        self.assertEqual(stat.S_IMODE(info.external_attr >> 16), 0o755)
                        self.assertEqual(archive.read(info), self.exe.read_bytes())
                original = output.read_bytes()
                with self.assertRaises(ValueError):
                    packaging.package(source, output)
                self.assertEqual(output.read_bytes(), original)

    def test_bundle_layout_tokens_licenses_hashes_and_unix_executable_modes(self):
        root_vars = {"codex": ".", "claude-code": "${CLAUDE_PLUGIN_ROOT}",
                     "claude-desktop": "${__dirname}", "gemini": "${extensionPath}",
                     "cursor": "${CURSOR_PLUGIN_ROOT}"}
        manifests = {"codex": "mcp.json", "claude-code": ".mcp.json",
                     "claude-desktop": "manifest.json", "gemini": "gemini-extension.json", "cursor": "mcp.json"}
        for platform in integrate.PLATFORMS:
            with self.subTest(platform=platform):
                output = self.base / ("bundle-" + platform)
                result = integrate.bundle(self.args(platform=platform, output=str(output)))
                expected = set(integrate.CLIENTS) - ({"claude-desktop"} if platform.startswith("linux") else set())
                self.assertEqual({item["client"] for item in result["bundles"]}, expected)
                binary = "screencue.exe" if platform.startswith("windows") else "screencue"
                for item in result["bundles"]:
                    client, archive_path = item["client"], Path(item["archive"])
                    self.assertEqual(item["sha256"], hashlib.sha256(archive_path.read_bytes()).hexdigest())
                    with zipfile.ZipFile(archive_path) as archive:
                        self.assertIsNone(archive.testzip())
                        names = archive.namelist()
                        self.assertIn("skills/screencue/SKILL.md", names)
                        self.assertTrue(all(not name.startswith("/") and ".." not in Path(name).parts for name in names))
                        self.assertFalse(any(".template." in name for name in names))
                        self.assertEqual(archive.read("bin/" + binary), self.exe.read_bytes())
                        executable = archive.getinfo("bin/" + binary)
                        self.assertEqual(executable.create_system, 3)
                        self.assertEqual(stat.S_IMODE(executable.external_attr >> 16), 0o755)
                        for license_name in ("LICENSE", "IBM-Plex-OFL.txt", "THIRD-PARTY-LICENSES.txt"):
                            self.assertIn("licenses/" + license_name, names)
                        for name in names:
                            if name.endswith((".json", ".md", ".yaml", ".toml", ".mdc")):
                                self.assertNotIn("__SCREENCUE_", archive.read(name).decode("utf-8-sig"), name)
                        manifest = json.loads(archive.read(manifests[client]))
                        if client == "claude-desktop":
                            server = manifest["server"]["mcp_config"]
                            self.assertEqual(manifest["server"]["entry_point"], "bin/" + binary)
                            self.assertEqual(manifest["compatibility"]["platforms"], ["win32" if platform.startswith("windows") else "darwin"])
                        else:
                            server = manifest["mcpServers"]["screencue"]
                        self.assertEqual(server["command"], root_vars[client] + "/bin/" + binary)
                        self.assertEqual(server["args"], ["--mcp"])
                        if client == "codex":
                            compatibility = json.loads(archive.read(".mcp.json"))["mcpServers"]["screencue"]
                            self.assertEqual(compatibility["command"], "${PLUGIN_ROOT}/bin/" + binary)
                            self.assertEqual(compatibility["args"], ["--mcp"])
                marketplace = json.loads((output / ".agents/plugins/marketplace.json").read_text(encoding="utf-8-sig"))
                self.assertTrue((output / marketplace["plugins"][0]["source"]["path"]).is_dir())
                if "claude-desktop" in expected:
                    with zipfile.ZipFile(output / "screencue-claude-desktop-skill.zip") as skill:
                        self.assertIn("screencue/SKILL.md", skill.namelist())
                else:
                    self.assertFalse((output / "screencue-claude-desktop-skill.zip").exists())
                archives_before = {path.name: path.read_bytes() for path in output.glob("*.zip")}
                with self.assertRaises(ValueError):
                    integrate.bundle(self.args(platform=platform, output=str(output)))
                self.assertEqual({path.name: path.read_bytes() for path in output.glob("*.zip")}, archives_before)


if __name__ == "__main__":
    unittest.main()
