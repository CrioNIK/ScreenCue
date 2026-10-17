#!/usr/bin/env python3
"""Configure and package ScreenCue integrations. Python 3.11+, standard library only."""
from __future__ import annotations

import argparse
import copy
import datetime
import hashlib
import json
import os
from pathlib import Path
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import tomllib
import uuid
import zipfile

REPO = Path(__file__).resolve().parents[1]
CLIENTS = ("codex", "claude-code", "claude-desktop", "gemini", "cursor")
PLATFORMS = ("windows-x64", "macos-apple-silicon", "macos-intel", "linux-x64-x11")
TOOLS = {"screen_displays", "screen_hint", "screen_clear", "screen_status"}


def read_json(path: Path) -> dict:
    value = json.loads(path.read_text(encoding="utf-8-sig")) if path.exists() else {}
    if not isinstance(value, dict):
        raise ValueError(f"Expected a JSON object: {path}")
    return value


def json_bytes(value: dict) -> bytes:
    return (json.dumps(value, ensure_ascii=False, indent=2) + "\n").encode("utf-8")


def executable_path(value: str) -> Path:
    path = Path(value).expanduser().resolve(strict=True)
    if not path.is_file():
        raise ValueError("--executable must name the ScreenCue binary")
    return path


def toml_value(value):
    if isinstance(value, bool):
        return "true" if value else "false"
    if isinstance(value, str):
        return json.dumps(value, ensure_ascii=False)
    if isinstance(value, (int, float)):
        return repr(value)
    if isinstance(value, list):
        return "[" + ", ".join(toml_value(item) for item in value) + "]"
    if isinstance(value, dict):
        return "{ " + ", ".join(f"{json.dumps(key)} = {toml_value(item)}" for key, item in value.items()) + " }"
    if isinstance(value, (datetime.datetime, datetime.date, datetime.time)):
        return value.isoformat()
    raise ValueError(f"Unsupported TOML value type: {type(value).__name__}")


def merge_codex(text: str, executable: Path) -> str:
    """Replace only our table; verify all other parsed TOML values are identical."""
    before = tomllib.loads(text)
    expected = copy.deepcopy(before)
    servers = expected.setdefault("mcp_servers", {})
    if not isinstance(servers, dict):
        raise ValueError("Codex mcp_servers must be a table")
    previous = servers.get("screencue", {})
    if not isinstance(previous, dict):
        raise ValueError("Codex mcp_servers.screencue must be a table")
    entry = {**previous, "command": str(executable), "args": ["--mcp"], "enabled": True}
    for key in ("url", "http_headers", "env_http_headers", "bearer_token_env_var"):
        entry.pop(key, None)
    servers["screencue"] = entry
    kept, dropping, found = [], False, False
    for line in text.splitlines(keepends=True):
        if re.match(r"^\s*\[", line):
            # A section-looking line inside a multiline string is caught by the
            # complete before/after data comparison below, never silently changed.
            dropping = bool(re.match(r'''^\s*\[\s*(?:mcp_servers|"mcp_servers"|'mcp_servers')\s*\.\s*(?:screencue|"screencue"|'screencue')\s*(?:\.|\])''', line))
            found |= dropping
        if not dropping:
            kept.append(line)
    if "screencue" in before.get("mcp_servers", {}) and not found:
        raise ValueError("ScreenCue uses an inline Codex table; edit it with Codex instead of rewriting unrelated settings")
    block = "[mcp_servers.screencue]\n" + "".join(f"{json.dumps(key)} = {toml_value(value)}\n" for key, value in entry.items())
    result = "".join(kept).rstrip() + "\n\n" + block
    if tomllib.loads(result) != expected:
        raise ValueError("Refusing a TOML update that changes unrelated settings")
    return result.lstrip("\n")


def config_path(client: str, scope: str, root: Path, home: Path, platform: str) -> Path | None:
    base = root if scope == "project" else home
    if client == "claude-desktop":
        if scope == "project" or platform.startswith("linux"):
            return None
        if platform.startswith("windows"):
            appdata = Path(os.environ.get("APPDATA", home / "AppData/Roaming")) if home == Path.home() else home / "AppData/Roaming"
            return appdata / "Claude/claude_desktop_config.json"
        return home / "Library/Application Support/Claude/claude_desktop_config.json"
    if client == "codex":
        codex_home = Path(os.environ.get("CODEX_HOME", home / ".codex")) if home == Path.home() else home / ".codex"
        return (root / ".codex" if scope == "project" else codex_home) / "config.toml"
    if client == "claude-code" and scope == "user" and home == Path.home() and os.environ.get("CLAUDE_CONFIG_DIR"):
        return Path(os.environ["CLAUDE_CONFIG_DIR"]) / ".claude.json"
    relative = {"claude-code": ".mcp.json" if scope == "project" else ".claude.json", "gemini": ".gemini/settings.json", "cursor": ".cursor/mcp.json"}
    return base / relative[client]


def native_platform() -> str:
    if sys.platform == "win32":
        return "windows-x64"
    if sys.platform == "darwin":
        import platform
        return "macos-apple-silicon" if platform.machine() == "arm64" else "macos-intel"
    return "linux-x64-x11"


def plan_configuration(clients, executable, scope, root, home, platform):
    plan, skipped = {}, []
    for client in clients:
        destination = config_path(client, scope, root, home, platform)
        if destination is None:
            skipped.append(f"{client}: no {scope}-scope configuration on {platform}; use its desktop bundle on a supported platform")
            continue
        if client == "codex":
            previous = destination.read_text(encoding="utf-8-sig") if destination.exists() else ""
            content = merge_codex(previous, executable).encode("utf-8")
        else:
            config = read_json(destination)
            servers = config.setdefault("mcpServers", {})
            if not isinstance(servers, dict):
                raise ValueError(f"mcpServers must be an object: {destination}")
            entry = servers.get("screencue", {})
            if not isinstance(entry, dict):
                raise ValueError(f"screencue must be an object: {destination}")
            entry = {**entry, "command": str(executable), "args": ["--mcp"]}
            # Preserve server-specific permissions and environment, but do not
            # leave a former HTTP transport competing with this stdio entry.
            for key in ("url", "httpUrl", "headers"):
                entry.pop(key, None)
            if client == "claude-code":
                entry["type"] = "stdio"
            elif entry.get("type") in ("http", "sse", "streamable-http"):
                entry.pop("type")
            servers["screencue"] = entry
            content = json_bytes(config)
        plan[destination] = content
        if client == "claude-desktop":
            continue  # Desktop skills are uploaded through its supported UI.
        folder = {"codex": ".agents", "claude-code": ".claude", "gemini": ".gemini", "cursor": ".cursor"}[client]
        base = root if scope == "project" else home
        skill_root = base / folder / "skills/screencue"
        if client == "claude-code" and scope == "user" and home == Path.home() and os.environ.get("CLAUDE_CONFIG_DIR"):
            skill_root = Path(os.environ["CLAUDE_CONFIG_DIR"]) / "skills/screencue"
        skill_source = REPO / "integrations" / client / "skills/screencue"
        if not (skill_source / "SKILL.md").is_file():
            raise ValueError(f"Missing packaged skill for {client}")
        for source in skill_source.rglob("*"):
            if source.is_file():
                plan[skill_root / source.relative_to(skill_source)] = source.read_bytes()
        if client == "cursor" and scope == "project":
            for source in (REPO / "integrations/cursor/rules").glob("*.mdc"):
                plan[root / ".cursor/rules" / source.name] = source.read_bytes()
    return plan, skipped


def write_safely(path: Path, data: bytes) -> str:
    if path.is_symlink():
        raise ValueError(f"Refusing to replace a symlink: {path}")
    if path.exists() and path.read_bytes() == data:
        return "unchanged"
    path.parent.mkdir(parents=True, exist_ok=True)
    existing_mode = path.stat().st_mode if path.exists() else 0o600
    if path.exists():
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        backup = path.with_name(path.name + f".screencue-backup-{stamp}-{uuid.uuid4().hex[:8]}")
        shutil.copy2(path, backup)
    fd, temporary = tempfile.mkstemp(prefix=".screencue-", dir=path.parent)
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(data)
        os.chmod(temporary, existing_mode)
        os.replace(temporary, path)
    finally:
        if os.path.exists(temporary):
            os.unlink(temporary)
    return "written"


def configure(args):
    exe = executable_path(args.executable)
    root, home = Path(args.root).resolve(), Path(args.home).resolve()
    clients = CLIENTS if args.client == "all" else (args.client,)
    plan, skipped = plan_configuration(clients, exe, args.scope, root, home, native_platform())
    if not plan:
        raise ValueError("; ".join(skipped))
    # Validate every destination before writing any client configuration.
    for destination in plan:
        if destination.is_symlink() or (destination.exists() and not destination.is_file()):
            raise ValueError(f"Unsupported configuration destination: {destination}")
    results = []
    for destination, data in plan.items():
        state = "planned" if args.dry_run else write_safely(destination, data)
        results.append({"path": str(destination), "status": state})
    return {"files": results, "skipped": skipped, "note": "Restart/reconnect the client. Client trust and permission prompts still apply. No client was launched and no approval policy was relaxed."}


def replace_tokens(value, replacements):
    if isinstance(value, dict):
        return {key: replace_tokens(item, replacements) for key, item in value.items()}
    if isinstance(value, list):
        return [replace_tokens(item, replacements) for item in value]
    if isinstance(value, str):
        for token, replacement in replacements.items():
            value = value.replace(token, replacement)
    return value


def zip_directory(source: Path, destination: Path, prefix=""):
    with zipfile.ZipFile(destination, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(source.rglob("*")):
            if path.is_file():
                name = prefix + path.relative_to(source).as_posix()
                info = zipfile.ZipInfo.from_file(path, arcname=name)
                info.create_system = 3
                mode = 0o100755 if path.parent.name == "bin" else 0o100644
                info.external_attr = mode << 16
                info.compress_type = zipfile.ZIP_DEFLATED
                archive.writestr(info, path.read_bytes())


def bundle(args):
    exe = executable_path(args.executable)
    output = Path(args.output).resolve()
    output.mkdir(parents=True, exist_ok=True)
    clients = CLIENTS if args.client == "all" else (args.client,)
    binary = "screencue.exe" if args.platform.startswith("windows") else "screencue"
    root_vars = {"codex": "${PLUGIN_ROOT}", "claude-code": "${CLAUDE_PLUGIN_ROOT}", "claude-desktop": "${__dirname}", "gemini": "${extensionPath}", "cursor": "${CURSOR_PLUGIN_ROOT}"}
    supported = [client for client in clients if client != "claude-desktop" or not args.platform.startswith("linux")]
    for client in supported:
        if (output / client).exists():
            raise ValueError(f"Bundle directory already exists; use a new output directory: {output / client}")
        suffix = ".mcpb" if client == "claude-desktop" else ".zip"
        if (output / f"screencue-{client}-{args.platform}{suffix}").exists():
            raise ValueError("Bundle archive already exists")
    result = []
    for client in supported:
        source, destination = REPO / "integrations" / client, output / client
        if not source.is_dir():
            raise ValueError(f"Missing integration source: {source}")
        replacements = {"__SCREENCUE_EXECUTABLE__": root_vars[client] + "/bin/" + binary, "__SCREENCUE_BINARY__": binary}
        for path in source.rglob("*"):
            if not path.is_file() or ".template." in path.name or path.name == "claude_desktop_config.json":
                continue
            if path.is_symlink():
                raise ValueError("Bundle sources must not be symlinks")
            target = destination / path.relative_to(source)
            target.parent.mkdir(parents=True, exist_ok=True)
            if path.suffix == ".json":
                value = replace_tokens(read_json(path), replacements)
                if client == "claude-desktop" and path.name == "manifest.json":
                    value.setdefault("compatibility", {})["platforms"] = ["win32" if args.platform.startswith("windows") else "darwin"]
                target.write_bytes(json_bytes(value))
            else:
                shutil.copy2(path, target)
        (destination / "bin").mkdir(exist_ok=True)
        shutil.copy2(exe, destination / "bin" / binary)
        (destination / "bin" / binary).chmod(0o755)
        (destination / "licenses").mkdir(exist_ok=True)
        for source_path, name in ((REPO / "LICENSE", "LICENSE"), (REPO / "assets/OFL.txt", "IBM-Plex-OFL.txt"), (REPO / "docs/THIRD-PARTY-LICENSES.txt", "THIRD-PARTY-LICENSES.txt")):
            shutil.copy2(source_path, destination / "licenses" / name)
        # MCPB's uploaded skill is separate; the Desktop bundle does not load it.
        suffix = ".mcpb" if client == "claude-desktop" else ".zip"
        archive = output / f"screencue-{client}-{args.platform}{suffix}"
        zip_directory(destination, archive)
        result.append({"client": client, "directory": str(destination), "archive": str(archive), "sha256": hashlib.sha256(archive.read_bytes()).hexdigest()})
        if client == "claude-desktop":
            zip_directory(destination / "skills/screencue", output / "screencue-claude-desktop-skill.zip", "screencue/")
    if "codex" in supported:
        template = REPO / "integrations/codex/marketplace.template.json"
        if template.exists():
            marketplace = output / ".agents/plugins/marketplace.json"
            marketplace.parent.mkdir(parents=True, exist_ok=True)
            marketplace.write_bytes(template.read_bytes())
    return {"bundles": result, "note": "Bundle validation does not prove authenticated end-to-end operation in every client. Claude Desktop has no supported Linux package."}


class McpProbe:
    def __init__(self, executable):
        flags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
        self.process = subprocess.Popen([str(executable), "--mcp"], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", creationflags=flags)
        self.lines, self.errors, self.sequence = queue.Queue(), [], 0
        def read_stdout():
            try:
                for line in self.process.stdout:
                    self.lines.put(line)
            finally:
                self.lines.put(None)
        def read_stderr():
            self.errors.append(self.process.stderr.read())
        self.stdout_thread = threading.Thread(target=read_stdout, daemon=True)
        self.stderr_thread = threading.Thread(target=read_stderr, daemon=True)
        self.stdout_thread.start()
        self.stderr_thread.start()

    def send(self, method, params=None, notification=False):
        value = {"jsonrpc": "2.0", "method": method}
        if params is not None:
            value["params"] = params
        if not notification:
            self.sequence += 1
            value["id"] = self.sequence
        self.process.stdin.write(json.dumps(value) + "\n")
        self.process.stdin.flush()
        if notification:
            return None
        try:
            line = self.lines.get(timeout=10)
        except queue.Empty:
            raise ValueError("ScreenCue MCP handshake timed out") from None
        if line is None:
            raise ValueError("ScreenCue exited before responding; run in an interactive desktop session")
        reply = json.loads(line)
        if reply.get("id") != self.sequence or reply.get("error"):
            raise ValueError(f"Invalid MCP reply: {reply}")
        return reply["result"]

    def close(self):
        if self.process.stdin and not self.process.stdin.closed:
            self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.kill()
            self.process.wait(timeout=5)
            raise ValueError("ScreenCue did not exit after stdin closed")
        finally:
            self.stdout_thread.join(timeout=1)
            self.stderr_thread.join(timeout=1)
            self.process.stdout.close()
            self.process.stderr.close()


def probe(executable):
    client = McpProbe(executable)
    try:
        initialized = client.send("initialize", {"protocolVersion": "2025-11-25", "capabilities": {}, "clientInfo": {"name": "screencue-integration-doctor", "version": "1"}})
        if initialized.get("serverInfo", {}).get("name") != "screencue":
            raise ValueError("The configured binary is not ScreenCue")
        client.send("notifications/initialized", notification=True)
        names = {tool["name"] for tool in client.send("tools/list")["tools"]}
        if names != TOOLS:
            raise ValueError(f"Unexpected ScreenCue tool set: {sorted(names)}")
        status_result = client.send("tools/call", {"name": "screen_status", "arguments": {}})
        status = json.loads(status_result["content"][0]["text"])
        if status_result.get("isError") or not status.get("ok") or status.get("visible"):
            raise ValueError("Expected a healthy, hidden ScreenCue instance")
        return {"server": initialized["serverInfo"], "tools": sorted(names), "visible": False}
    finally:
        client.close()
        if client.process.returncode != 0 or any(client.errors):
            raise ValueError("ScreenCue reported an error on shutdown")


def doctor(args):
    clients = CLIENTS if args.client == "all" else (args.client,)
    results = []
    for client in clients:
        path = config_path(client, args.scope, Path(args.root).resolve(), Path(args.home).resolve(), native_platform())
        if path is None:
            results.append({"client": client, "status": "not supported for this scope/platform"})
            continue
        if not path.exists():
            raise ValueError(f"Configuration not found: {path}")
        if client == "codex":
            servers = tomllib.loads(path.read_text(encoding="utf-8-sig")).get("mcp_servers", {})
        else:
            servers = read_json(path).get("mcpServers", {})
        if not isinstance(servers, dict):
            raise ValueError(f"Expected an MCP server object: {path}")
        entry = servers.get("screencue")
        if not isinstance(entry, dict) or entry.get("args") != ["--mcp"] or not isinstance(entry.get("command"), str):
            raise ValueError(f"ScreenCue stdio entry is missing or has unexpected arguments: {path}")
        if entry.get("enabled") is False or entry.get("disabled") is True:
            raise ValueError(f"ScreenCue is disabled in client configuration: {path}")
        if any(key in entry for key in ("url", "httpUrl")) or entry.get("type", "stdio") != "stdio":
            raise ValueError(f"Expected a local stdio transport: {path}")
        result = probe(executable_path(entry["command"]))
        results.append({"client": client, "configuration": str(path), "status": "MCP protocol passed", **result})
    return {"checks": results, "note": "These checks launch the configured MCP binary, not the AI client. Client authentication, trust, screen observation and model tool selection need separate end-to-end checks."}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="operation", required=True)
    for name in ("configure", "doctor", "bundle"):
        child = commands.add_parser(name)
        child.add_argument("--client", choices=(*CLIENTS, "all"), required=True)
        if name != "doctor":
            child.add_argument("--executable", required=True)
        if name == "bundle":
            child.add_argument("--output", required=True)
            child.add_argument("--platform", choices=PLATFORMS, default=native_platform(), help="Target platform (default: this machine); supply its matching binary")
        else:
            child.add_argument("--scope", choices=("project", "user"), default="project")
            child.add_argument("--root", default=str(Path.cwd()))
            child.add_argument("--home", default=str(Path.home()), help="Alternate home for isolated configuration tests")
        if name == "configure":
            child.add_argument("--dry-run", action="store_true")
    args = parser.parse_args(argv)
    try:
        result = {"configure": configure, "doctor": doctor, "bundle": bundle}[args.operation](args)
    except (OSError, ValueError, KeyError, TypeError, subprocess.SubprocessError) as error:
        print(f"ScreenCue integration error: {error}", file=sys.stderr)
        return 1
    print(json.dumps(result, ensure_ascii=False, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
