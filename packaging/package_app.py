#!/usr/bin/env python3
"""Stage native desktop applications without downloading or installing anything."""
from __future__ import annotations

import argparse
import json
import plistlib
import shutil
import subprocess
import tarfile
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def copy(source: Path, destination: Path) -> None:
    if not source.is_file():
        raise FileNotFoundError(f"Required package input is missing: {source}")
    destination.parent.mkdir(parents=True, exist_ok=True)
    shutil.copy2(source, destination)


def package(platform: str, overlay: Path, helper: Path, output: Path) -> dict:
    version = tomllib.loads((ROOT / "Cargo.toml").read_text(encoding="utf-8"))["package"]["version"]
    output = output.resolve()
    stage = output / platform
    if stage.exists():
        raise FileExistsError(f"Package staging directory must be new: {stage}")
    if platform.startswith("macos-"):
        app = stage / "ScreenCue.app"
        binaries = app / "Contents/MacOS"
        resources = app / "Contents/Resources"
    else:
        app = stage / "ScreenCue"
        binaries = resources = app
    suffix = ".exe" if platform.startswith("windows-") else ""
    for source, name in [(overlay, "screencue"), (helper, "screencue-listen")]:
        destination = binaries / (name + suffix)
        copy(source.resolve(), destination)
        if not suffix:
            destination.chmod(0o755)
    copy(ROOT / "LICENSE", resources / "LICENSE")
    copy(ROOT / "docs/INSTALL.md", resources / "README.md")
    copy(ROOT / "docs/CAPTURE-PRIVACY.md", resources / "docs/CAPTURE-PRIVACY.md")
    copy(ROOT / "assets/OFL.txt", resources / "licenses/IBM-Plex-OFL.txt")
    copy(ROOT / "docs/THIRD-PARTY-LICENSES.txt", resources / "licenses/THIRD-PARTY-LICENSES.txt")
    # Explicit public inputs only: never sweep local provider configuration or
    # credentials into a release archive with a wildcard.
    for name in ["README.md", "example.json"]:
        copy(ROOT / "conversation" / name, resources / "conversation" / name)
    # Scripts belong to Resources in an Apple bundle. Contents/MacOS is reserved
    # for signed native executables; placing Python files there causes strict
    # code-signature verification to treat them as unsigned nested code.
    for name in ["screencue_controller.py", "conversation_orchestrator.py", "CONVERSATION.md", "conversation.example.json"]:
        copy(ROOT / "controller" / name, resources / "controller" / name)
    helper_notices = ROOT / "conversation/THIRD-PARTY-LICENSES.txt"
    copy(helper_notices, resources / "licenses/CONVERSATION-THIRD-PARTY-LICENSES.txt")
    copy(ROOT / "assets/app-icon-256.png", resources / "app-icon.png")
    metadata = {"name": "ScreenCue", "version": version, "platform": platform,
                "audio_helper": True, "signed": False}
    (resources / "package.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    artifact = None
    if platform.startswith("macos-"):
        copy(ROOT / "assets/app-icon.icns", resources / "ScreenCue.icns")
        with (app / "Contents/Info.plist").open("wb") as stream:
            plistlib.dump({
                "CFBundleName": "ScreenCue", "CFBundleDisplayName": "ScreenCue",
                "CFBundleIdentifier": "com.screencue.desktop", "CFBundleExecutable": "screencue",
                "CFBundlePackageType": "APPL", "CFBundleIconFile": "ScreenCue.icns",
                "CFBundleShortVersionString": version, "CFBundleVersion": version,
                "LSMinimumSystemVersion": "14.2", "NSHighResolutionCapable": True,
                "NSMicrophoneUsageDescription": "ScreenCue listens to your microphone only when you start conversation assistance.",
                "NSAudioCaptureUsageDescription": "ScreenCue listens to meeting audio only when you enable system audio for conversation assistance.",
                "LSApplicationCategoryType": "public.app-category.productivity",
            }, stream)
        # An ad-hoc signature allows the bundled executables to retain one
        # coherent bundle identity. It is not Developer ID signing/notarization.
        subprocess.run(["codesign", "--force", "--sign", "-", str(binaries / "screencue-listen")], check=True)
        subprocess.run(["codesign", "--force", "--sign", "-", str(app)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
        applications = stage / "Applications"
        if not applications.exists():
            applications.symlink_to("/Applications")
        artifact = output / f"ScreenCue-{version}-{platform}.dmg"
        subprocess.run(["hdiutil", "create", "-volname", "ScreenCue", "-srcfolder", str(stage),
                        "-ov", "-format", "UDZO", str(artifact)], check=True)
        subprocess.run(["hdiutil", "verify", str(artifact)], check=True)
    elif platform.startswith("linux-"):
        for name in ["install.sh", "uninstall.sh", "screencue.desktop"]:
            copy(ROOT / "packaging/linux" / name, app / name)
        for name in ["install.sh", "uninstall.sh"]:
            (app / name).chmod(0o755)
        artifact = output / f"ScreenCue-{version}-{platform}-app.tar.gz"
        def archive_mode(member: tarfile.TarInfo) -> tarfile.TarInfo:
            if member.isfile() and Path(member.name).name in {"screencue", "screencue-listen", "install.sh", "uninstall.sh"}:
                member.mode |= 0o111
            return member
        with tarfile.open(artifact, "w:gz") as archive:
            archive.add(app, arcname="ScreenCue", filter=archive_mode)
    return {"stage": str(app), "version": version, "artifact": str(artifact) if artifact else None}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", required=True, choices=["windows-x64", "macos-apple-silicon", "macos-intel", "linux-x64-x11"])
    parser.add_argument("--overlay", type=Path, required=True)
    parser.add_argument("--helper", type=Path, required=True)
    parser.add_argument("--output", type=Path, default=ROOT / "dist/app")
    args = parser.parse_args()
    print(json.dumps(package(args.platform, args.overlay, args.helper, args.output)))


if __name__ == "__main__":
    main()
