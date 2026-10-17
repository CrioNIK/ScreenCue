#!/usr/bin/env python3
"""Collect generated native plugins into one release ZIP, without duplicate ZIPs."""
import argparse
from pathlib import Path
import zipfile

from integrate import REPO


def package(source: Path, output: Path):
    if output.exists():
        raise ValueError(f"Archive already exists: {output}")
    if not (source / "codex/plugin.json").is_file():
        raise ValueError("Expected the output of integrate.py bundle --client all")
    output.parent.mkdir(parents=True, exist_ok=True)
    with zipfile.ZipFile(output, "w", compression=zipfile.ZIP_DEFLATED) as archive:
        for path in sorted(source.rglob("*")):
            if not path.is_file():
                continue
            relative = path.relative_to(source)
            # Desktop installs the MCPB; the loose duplicate is unnecessary.
            if relative.parts[0] == "claude-desktop":
                continue
            if len(relative.parts) == 1 and path.suffix == ".zip" and path.name != "screencue-claude-desktop-skill.zip":
                continue
            info = zipfile.ZipInfo.from_file(path, arcname=relative.as_posix())
            info.create_system = 3
            info.external_attr = (0o100755 if path.parent.name == "bin" else 0o100644) << 16
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, path.read_bytes())
        for path in (REPO / "docs/clients").glob("*.md"):
            archive.write(path, "docs/clients/" + path.name)
        archive.write(REPO / "docs/INTEGRATION.md", "docs/INTEGRATION.md")
        archive.write(REPO / "docs/VERIFICATION.md", "docs/VERIFICATION.md")
        archive.writestr("README.md", "# ScreenCue client integrations\n\n"
            "Keep this extracted folder in a permanent location. It contains native plugins and their binaries. "
            "Python and Node.js are not required to run these packages.\n\n"
            "- Codex: `codex plugin marketplace add /absolute/path/to/this-folder`, then `codex plugin add screencue@screencue-local`.\n"
            "- Claude Code: `claude --plugin-dir /absolute/path/to/this-folder/claude-code`.\n"
            "- Gemini CLI: `gemini extensions install /absolute/path/to/this-folder/gemini`.\n"
            "- Cursor: copy the `cursor` folder to `~/.cursor/plugins/local/screencue`, then reload Cursor.\n"
            "- Claude Desktop (Windows/macOS): install the `.mcpb` in Extensions and upload the skill ZIP separately.\n\n"
            "Choose the package matching your platform. Follow [the client guides](docs/clients/) and normal client trust prompts. "
            "Use one connection per client. Screen observation is a separate client capability; these packages do not grant it. "
            "See [validation scope](docs/VERIFICATION.md).\n")
    return output


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--input", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    print(package(args.input.resolve(), args.output.resolve()))
