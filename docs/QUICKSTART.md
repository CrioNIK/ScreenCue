# ScreenCue 0.0.3

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

Text is an independent dark brush panel. Use `title` for its heading and `text` for its body; the panel adapts to content and paginates long text. Set `arrow: false` and `target: false` for a panel alone. Agents can choose `panel_x`, `panel_y`, `panel_width`, and `text_page`. Plain text is bounded to 16,000 body characters; embedded fonts cover Latin and Cyrillic.

Supported platforms: Windows x64; macOS 12.3+ (local automatic color requires 15.2+ and existing permission); Linux x64/X11 with a compositor. Native Wayland is not supported. This is a preview release. Windows desktop behavior has been tested; macOS/Linux have build and unit-test coverage. Client guides state which integrations have been tested live.

An ordinary desktop overlay cannot guarantee visibility over every true exclusive-fullscreen game. Check `presentation` diagnostics and the [fullscreen plan](https://github.com/CrioNIK/ScreenCue/blob/main/docs/FULLSCREEN-PLAN.md). No game-specific compatibility is claimed by this release.

[Source, options, and limitations](https://github.com/CrioNIK/ScreenCue). Code: MIT (`LICENSE`). Embedded IBM Plex fonts: SIL OFL (`licenses/IBM-Plex-OFL.txt`). Dependency notices: `licenses/THIRD-PARTY-LICENSES.txt`.
