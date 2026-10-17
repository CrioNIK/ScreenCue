# ScreenCue 0.2.0

AI-guided arrows over your desktop. One native executable, with no Rust, Node.js, or .NET installation required to run it.

1. Keep the extracted folder in a permanent location.
2. Run `screencue demo` (`.\screencue.exe demo` in Windows PowerShell). Hide the hint with **Ctrl+Alt+X**, or **Ctrl+Option+X** on macOS.
3. Connect your agent using the [integration guide](https://github.com/CrioNIK/ScreenCue/blob/main/docs/INTEGRATION.md) and [client guides](https://github.com/CrioNIK/ScreenCue/tree/main/docs/clients).

For example, connect Codex on Windows:

```powershell
codex mcp add screencue -- "C:\Tools\ScreenCue\screencue.exe" --mcp
```

Replace the path with your own. On macOS/Linux, use the absolute path to `screencue`. Start a new client session after installation.

The hint lets clicks pass through, never takes focus, and follows its original window. If the target goes offscreen or the window is minimized, a labeled pointer stays at the display edge. Scrolling, resizing, and interface reflow require fresh coordinates from the agent. The client provides voice and screen observation; ScreenCue draws the supplied hint. Local MCP configuration alone does not give a cloud or voice session desktop access.

Supported platforms: Windows x64; macOS 12.3+ (local automatic color requires 15.2+ and existing permission); Linux x64/X11 with a compositor. Native Wayland is not supported. This is a preview release. Windows desktop behavior has been tested; macOS/Linux have build and unit-test coverage. Client guides state which integrations have been tested live.

[Source, options, and limitations](https://github.com/CrioNIK/ScreenCue). Code: MIT (`LICENSE`). Embedded IBM Plex Mono: SIL OFL (`licenses/IBM-Plex-Mono-OFL.txt`). Dependency notices: `licenses/THIRD-PARTY-LICENSES.txt`.
