# Install ScreenCue

Download the application for your operating system from
[GitHub Releases](https://github.com/CrioNIK/ScreenCue/releases). The app packages
include the settings window, desktop overlay, and optional conversation helper.
Portable overlay archives and agent integration bundles remain separate downloads.
The optional System 1/System 2 conversation orchestrator is included under
`controller/`; it needs a separately installed Python interpreter configured by
absolute path. The basic native helper does not require Python.
On macOS, controller scripts and their examples are in
`ScreenCue.app/Contents/Resources/controller`; the two native executables are in
`Contents/MacOS`. Windows and Linux keep `controller/` beside the executables.

| Download | Installation |
| --- | --- |
| `ScreenCue-0.0.4-windows-x64-setup.exe` | Run the installer. It installs for your user and adds ScreenCue to the Start menu. A desktop shortcut is optional. |
| `ScreenCue-0.0.4-macos-apple-silicon.dmg` | Open the disk image and drag ScreenCue to Applications. Requires macOS 14.2 or newer. |
| `ScreenCue-0.0.4-macos-intel.dmg` | The same installation for Intel Macs. Requires macOS 14.2 or newer. |
| `ScreenCue-0.0.4-linux-x64-x11-app.tar.gz` | Extract the archive, open its `ScreenCue` directory in a terminal, and run `sh install.sh`. |

The application opens its settings without starting the microphone or meeting
audio. Configure your provider and enable the audio sources you want before
starting conversation assistance. You can also use visual hints directly from
an MCP agent without the conversation helper.

Use **Connections → Models & keys** to configure each of the four model roles.
**Runtime & audio** selects managed connections, audio sources, speaker labels,
and the optional Python interpreter for independent planning/decision lanes.
The Conversation tab still accepts an external JSON configuration when managed
connections are off. See [Connections](CONNECTIONS.md) and
[API key storage](CREDENTIAL-STORAGE.md). Saving preferences or a key never starts
audio or contacts a model provider.

## Windows

The installer targets Windows 10 build 19041 or newer and Windows 11. It uses
`%LOCALAPPDATA%\Programs\ScreenCue`, does not require administrator access, and
does not add an automatic startup task. The installer and installed executables
carry the ScreenCue icon. This release is unsigned; Windows may identify the
publisher as unknown. Code signing is not claimed.

Open **ScreenCue** from the Start menu. To update, run a newer installer. To
remove it, use Windows **Installed apps** or **Uninstall ScreenCue** in the Start
menu. Uninstalling preserves personal settings and provider configuration.

Capture exclusion uses Windows' native window policy. A successfully applied
policy is not proof that every screen-sharing application respects it. See
[Capture privacy](CAPTURE-PRIVACY.md) and verify your actual sharing preview.

## macOS

Choose the download for your Mac's processor. The app is ad-hoc signed for bundle
integrity, not Developer ID signed or notarized. macOS may require you to allow
the downloaded application through **System Settings → Privacy & Security** after
your first attempt to open it. ScreenCue does not disable Gatekeeper.

Microphone access is requested only when you start a microphone session. Native
system-audio capture requires macOS 14.6 or newer and its own permission. The
app's minimum version is 14.2 because it includes the optional audio helper; the
separate overlay-only archive has its own lower deployment target.

ScreenCue cannot exclude its window from arbitrary ScreenCaptureKit captures on
macOS. Sharing one selected application window is an alternative that should be
verified in the receiver's view. To uninstall, move ScreenCue from Applications
to the Trash; personal configuration remains separate.

## Linux

The app currently requires an X11 session with compositing. Native Wayland is
not implemented. Install runtime libraries provided by your distribution,
including X11/XKB/XCB, ALSA, and OpenSSL 3. The release is built on the current
Ubuntu GitHub runner and may require a newer glibc than older distributions.

`install.sh` installs to `${XDG_DATA_HOME:-$HOME/.local/share}/ScreenCue` and adds
a user application-menu entry and icon. It does not use `sudo`, alter your PATH,
or install a background service. Start ScreenCue from your application menu.
System audio needs a selected PipeWire/PulseAudio monitor exposed as an input
device; a universal automatic loopback source is not assumed.

To uninstall, run the `uninstall.sh` inside the installed ScreenCue directory.
It removes that application directory, its menu entry, and its icon while
preserving external preferences. Capture exclusion is unavailable on this
backend; verify a selected-window sharing workflow if you need separate notes.

## Build application packages

Build both Rust crates for the same target:

```sh
cargo build --release --locked
cargo build --release --locked --manifest-path conversation/Cargo.toml
```

Run `packaging/package_app.py` with `--platform`, `--overlay`, and `--helper`.
It checks required binaries, icon assets, and license notices before staging an
application. macOS packaging must run on macOS because it uses `codesign` and
`hdiutil`. The Windows stage is compiled by
`packaging/windows/build-installer.ps1`; that script downloads the pinned Inno
Setup 6.7.3 compiler and validates its expected Authenticode publisher before
installing the build tool. Windows release resources are required in MSVC CI;
GNU development builds can proceed with a warning when a resource compiler is
unavailable.

CI preserves the existing portable archives and integration bundles and adds
the native app installer/image for each platform. No installer starts audio,
stores API keys, registers an agent client, or changes a meeting application.
