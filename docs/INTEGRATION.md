# Integrating ScreenCue

ScreenCue is a local desktop renderer with two stdin/stdout interfaces. It provides no model, screen-understanding service, screenshot tool, or clicking API. An agent observes the target application separately, then sends coordinates for one cue that stays visible until cleared or replaced.

The intended interaction is short: the user asks where to click, the agent calls `screen_hint`, and the user acts in the application already in front of them. The overlay passes through pointer input and does not take keyboard focus. A cue can have no label or a few words; it does not require opening a chat window. Voice and observation capabilities belong to the host client.

## Install and connect

Download the matching binary archive from [ScreenCue Releases](https://github.com/CrioNIK/ScreenCue/releases), extract it to a permanent location, and run `screencue demo` there (`.\screencue.exe demo` in Windows PowerShell). Keep the included licenses. macOS/Linux may need `chmod +x ./screencue`.

The following commands register a user-level local server. They write client configuration; run the command for the client you use. Replace the executable path with your installation path. On macOS/Linux, substitute `/absolute/path/screencue`.

### Codex

```powershell
codex mcp add screencue -- "C:\Tools\ScreenCue\screencue.exe" --mcp
codex mcp get screencue
```

Codex stores this in `~/.codex/config.toml`; local desktop, CLI, and IDE clients share MCP configuration on the same Codex host. Start a new session and check `/mcp`. Local CLI help was checked alongside the [official Codex MCP guide](https://developers.openai.com/codex/mcp/). A particular voice surface must expose the connected tools to the agent for it to use them.

### Dot / Alt: direct voice control

The intended flow is **the current voice agent observes the screen and calls ScreenCue directly**. Showing one cue must not require creating another chat, messaging an existing task, or handing the user to another agent. Passive observation must leave the user's foreground window unchanged. If the voice session lacks a permitted passive screen tool or ScreenCue, report the missing capability instead of opening windows or delegating the pointing request.

Registering a local MCP server in Codex does not establish that it is available to a separate Dot session. OpenAI documents Dot as a cloud agent with a separate local-computer connection and environment-dependent plugin access. Connecting that computer is separate from Codex's host connection. See [Dot computers and apps](https://learn.chatgpt.com/docs/dots/computers-and-apps). Voice inside an existing Codex task is a different surface and can use that task's conversation; see [ChatGPT Voice](https://learn.chatgpt.com/docs/features/voice).

Direct Dot support is **not yet verified**. ScreenCue supplies local stdio tools, not a cloud relay, voice engine, or a mechanism to add tools to another application's voice session. Do not advertise direct Dot support merely because a delegated Codex task successfully displayed an arrow.

To verify a direct integration, inspect the actual tool list in the Dot voice session, call `screen_displays` there, then use a fresh passive observation and call `screen_hint` from the same session. Confirm the cue appears, no other chat is created or messaged, and no window is activated. Call `screen_clear` from that same session. Tool availability, visible drawing, and the absence of delegation must all be checked.

### Claude Code

```powershell
claude mcp add --scope user --transport stdio screencue -- "C:\Tools\ScreenCue\screencue.exe" --mcp
claude mcp get screencue
```

Check `/mcp` in a new Claude Code session. User scope makes this registration available across your projects. The `--` separates Claude's options from the executable and its arguments. See the [official Claude Code MCP guide](https://code.claude.com/docs/en/mcp#option-3-add-a-local-stdio-server). These commands configure Claude Code; other Claude applications may use different setup paths.

### Gemini CLI

```powershell
gemini mcp add --scope user --transport stdio screencue "C:\Tools\ScreenCue\screencue.exe" -- --mcp
gemini mcp list
```

This writes the user MCP entry to `~/.gemini/settings.json`. Check `/mcp` in a new session. The final `-- --mcp` passes ScreenCue's flag to the subprocess. Local CLI help and the [official Gemini CLI MCP guide](https://geminicli.com/docs/tools/mcp-server/#adding-a-server-gemini-mcp-add) describe this syntax. This configures Gemini CLI, not the Gemini website.

No client configuration is included in the repository or applied by ScreenCue itself. These are documented setup commands, not claims that every client/version has been tested end to end.

## MCP launch configuration

Configure your client's local MCP transport to launch the platform's executable:

| Setting | Value |
| --- | --- |
| Command | An absolute path to `screencue.exe` on Windows or `screencue` on macOS/Linux |
| Arguments | `["--mcp"]` |
| Transport | Local stdio |
| Environment | The same interactive desktop session as the application being explained |

A commonly used JSON configuration shape is:

```json
{
  "mcpServers": {
    "screencue": {
      "command": "C:\\Tools\\ScreenCue\\screencue.exe",
      "args": ["--mcp"]
    }
  }
}
```

For a client using TOML server entries, the equivalent launch information is:

```toml
[mcp_servers.screencue]
command = "/absolute/path/screencue"
args = ["--mcp"]
```

These are configuration patterns, not a promise that every client accepts the same file or keys. Use your client's current documentation for the configuration location and registration command. No API key is required by ScreenCue. A cloud-only chat cannot launch this local executable without an appropriate local agent or bridge.

The macOS binary requires macOS 12.3 or later. Local automatic color sampling requires macOS 15.2 or later; on older supported versions, provide an agent palette or use an explicit color.

The server implements MCP revisions `2025-11-25`, `2025-06-18`, `2025-03-26`, and `2024-11-05`; an unsupported initialization revision receives `2025-11-25` for negotiation. Its framing follows the [MCP stdio transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports): UTF-8 JSON-RPC messages, one message per line, protocol output only on stdout. Diagnostics use stderr. Maximum message content is 32,768 bytes.

## Tools

| Tool | Purpose |
| --- | --- |
| `screen_displays` | Return physical monitor origins, dimensions, and scale factors |
| `screen_hint` | Replace this process's cue |
| `screen_clear` | Hide this process's cue |
| `screen_status` | Return visibility, current hint, process ID, and color source |

Each running `--mcp` process owns its own overlay. A default hint has no expiry timer and remains through clicks and active-window changes. The agent calls `screen_clear` when it should disappear or `screen_hint` to replace it. The user can also press the hide shortcut. Closing stdin ends the owning process and removes its overlay as normal session cleanup. There is no shared daemon, socket, or tray. Separate agents may launch separate instances; coordinate ownership in your client if you want only one cue on screen.

The global hide shortcut is `Ctrl+Alt+X` (`Ctrl+Option+X` on macOS). Override it at process launch, for example `screencue --mcp --hide-shortcut "Ctrl+Shift+X"`. Include Ctrl, Alt/Option, or Super/Command; plain letters and Shift-only shortcuts are rejected. The shortcut is registered only while a cue is visible and released on clear, expiry, or shutdown. `show` replies and status expose `hide_shortcut`, `hide_shortcut_id`, `shortcut_active`, and `shortcut_error`. If another application owns the combination, the cue can still be shown but the conflict is reported; ScreenCue does not silently choose another shortcut. `screen_clear` remains available. A successful shortcut hides the entire cue.

### Hint arguments

| Field | Meaning |
| --- | --- |
| `x`, `y` | Required integer coordinates in physical desktop pixels |
| `text` | Up to 500 characters; `""` hides the text card; default `"Click here"` |
| `style` | `"arrow"` by default; `"box"` uses a top-left anchor and requires `width`/`height` |
| `shape` | `"ellipse"`, `"rectangle"`, `"rounded"`, or `"polygon"`; defaults to ellipse for arrow style and rectangle for box style |
| `width`, `height` | Outline dimensions, 1–4,000 pixels; default 16 × 16 for arrow style |
| `arrow` | Boolean; defaults to true for arrow style and false for box style |
| `arrow_scale` | Number from 0.75 to 2.5, default 1.6; scales the prominent curved arrow while keeping a gap before the target |
| `target` | Boolean controlling the outline independently of the arrow; defaults to true |
| `animate` | Boolean; true by default: the four-frame SKETCH arrow advances at 5 steps/s and the light target contour animates at about 12 fps; false freezes both |
| `follow_window` | Boolean; true by default: follows the original native window beneath the target. False keeps desktop coordinates fixed |
| `points` | For polygon: 3–64 absolute desktop points, each `{ "x": integer, "y": integer }` |
| `color` | `"auto"` or an exact `"#RRGGBB"` value |
| `background_colors` | Optional array of up to 16 `"#RRGGBB"` colors observed by the agent; used by automatic color selection before attempting local sampling |
| `duration_ms` | Default 0: no expiry; stays until replaced, cleared, hidden with the shortcut, or process exit. Use 500–300,000 milliseconds only for an explicitly requested timer |
| `step` | Optional number, 1–99; default 1 |

With arrow style, `x,y` is the center of the target. With box style, it is the top-left corner, except for polygons, where it must lie inside the supplied outline. Polygon points stay absolute; they are not offsets from `x,y`. Explicitly sized targets and polygons must fit on one monitor. The arrow stops outside the target outline, leaving the marked control visible.

Examples of `screen_hint` arguments:

```json
{"x":600,"y":400,"text":"Open settings","shape":"rounded","width":120,"height":40,"color":"#AC4DFF"}
```

```json
{"x":600,"y":400,"text":"","shape":"ellipse","width":50,"height":30,"arrow":false,"target":true,"color":"#FFE35A"}
```

```json
{"x":600,"y":400,"text":"Choose this area","shape":"polygon","points":[{"x":550,"y":350},{"x":670,"y":360},{"x":650,"y":450},{"x":560,"y":440}],"arrow":true,"target":true}
```

Automatic color prefers red `#EF232D` when it has sufficient contrast across the observed background. It selects another color when red blends in and can return to red as the background changes. A manual hex color stays fixed.

With `auto`, a nonempty `background_colors` palette takes priority over local sampling. An agent can provide colors from its fresh observation, including on macOS:

```json
{"x":600,"y":400,"text":"Open settings","color":"auto","background_colors":["#FAFAFA","#202124","#E0E0E0"]}
```

Without a supplied palette, `auto` samples bounded patches up to 64 × 64 pixels, alternating between the target and arrow shaft at approximately one patch per second while visible. It combines the two recent regions and excludes pixels covered by its own drawn lines. This lets a large arrow account for background changes both near its target and along its shaft. Windows and X11 use native pixel reads. macOS uses ScreenCaptureKit on macOS 15.2+ if screen-recording permission is already available. ScreenCue checks permission without triggering a permission prompt; unavailable capture produces an explicit fallback reason.

Samples are transient and never saved or transmitted. A successful response includes the selected `color`, `color_source` (`client_palette`, `local_pixels`, `manual`, or `fallback`), and an optional `color_note`. If a later sample is pending or fails, status reports `local_pixels_stale` with a note while retaining the previous color. macOS capture is asynchronous: pending completion may be checked every 250 ms without starting captures more frequently. An initial pending fallback can become a local sample on a later refresh. A supplied palette is refreshed by sending a new hint with updated colors; it does not trigger local sampling. An explicit hex color also avoids sampling. The CLI equivalent is `--background-colors "#FAFAFA,#202124"`; arrow size can be set with `--arrow-scale 1.6`.

## Coordinates and observation

Call `screen_displays` before mapping a screenshot. Coordinates refer to the physical virtual desktop and can be negative for monitors to the left of or above the primary monitor. They are not browser CSS coordinates or macOS logical points.

For a screenshot of a known physical desktop rectangle, map an image point `(u,v)` as:

```text
x = desktop_left + u * captured_physical_width  / image_width
y = desktop_top  + v * captured_physical_height / image_height
```

Round to integer pixels. For a crop, include its physical origin. Obtain capture dimensions and scale from the actual observation tool; do not assume that the monitor scale factor alone describes a resized screenshot. Window movement is tracked locally by default. Refresh the observation after scrolling, zoom changes, or controls rearranging within the window.

### Window tracking

`follow_window: true` captures the native window under the initial target and retains that identity. Moving it translates the target, polygon points, and cue without another agent call; it does not attach to a different window passing underneath. A move that preserves the fitted geometry translates the existing native bitmap without rerendering. Local window-state polling is scheduled every 16 ms on Windows or 33 ms on macOS/X11 while anchored. These intervals are not guaranteed end-to-end latency; static anchored cues still perform metadata queries.

When the target leaves the screen or its window is minimized or closed, the cue stays visible at a screen edge with a note. A resized window produces a `needs_refresh` note until the agent supplies a new observation or the original size returns. Scrolling and internal reflow are not recognized. Use `follow_window: false` or CLI `--follow-window false` for fixed desktop coordinates; `demo` uses fixed coordinates.

Legacy Windows applications can report unchanged DPI between monitors. If the system rescales such a window, ScreenCue asks for a fresh target instead of assuming that the old coordinates still fit.

`screen_status` exposes the effective translated `hint`, `rendered_target: [x,y]` for the actually drawn anchor, and `arrow_tip: [x,y]` when an arrow is present. These can differ while a cue is pinned. `tracking.window_bounds: [x,y,width,height]` and `tracking.window_scale` provide native-window diagnostics. Inspect `tracking.state` and `tracking.note`:

| State | Meaning |
| --- | --- |
| `window` | Following the captured window normally |
| `offscreen`, `minimized`, `closed` | Visible edge cue explaining why the target is unavailable |
| `needs_refresh` | Window size changed; provide a new target observation |
| `fixed` | Fixed desktop coordinates, or no active cue after clear |
| `unavailable` | No native anchor could be captured; inspect the note and refresh as needed |

Suggested agent instruction:

> When I ask where to click, inspect the current screen with an authorized observation tool. Read ScreenCue's display geometry and map the target to physical desktop pixels. Immediately call screen_hint for one step, with text="" or a two-to-three-word label. Keep the current application in front; do not navigate, switch windows, or click for me. Give only a short accompanying sentence, then let me act. Leave duration_ms at its persistent default; do not add a timer unless requested. Use screen_clear when I finish the step or the cue is no longer relevant, and refresh the observation before the next step. Showing a cue does not mean the action was completed.

For example, after mapping a control center to `(900,420)`, one `screen_hint` call is enough:

```json
{"x":900,"y":420,"text":""}
```

The drawing call returns promptly while the cue remains visible. Send the next hint through the same connection to replace it; use `screen_clear` when the user finishes or the cue is no longer relevant. Keep the accompanying explanation brief.

## JSON transport for other agents

Launch `screencue --json` and keep stdin open. Write one UTF-8 JSON object per line and read one reply per line. This mode is not JSON-RPC and requires no handshake.

```json
{"command":"displays"}
{"command":"show","x":600,"y":400,"text":"Open settings","color":"#AC4DFF","duration_ms":0}
{"command":"status"}
{"command":"clear"}
```

`show` accepts the same hint fields as MCP. Replies contain `ok`; failures also contain `error`. Close stdin to shut down the process. A shell pipeline that immediately reaches EOF will also immediately remove the cue; use a persistent child process for an interactive agent.

## Command line

```sh
screencue displays
screencue demo
screencue show --x 600 --y 400 --text "Open settings"
screencue show --x 600 --y 400 --shape polygon --points "550,350;670,360;650,450;560,440" --text "Choose this area"
```

Each `show` command starts its own process, prints its result, and stays alive until the hide shortcut or process termination. Duration defaults to 0. Add `--duration-ms 8000` only when an eight-second expiry is wanted. A separate CLI invocation cannot clear a different process's cue; use MCP/JSON `clear` within the connection that created it.

## Platform notes and testing

- Windows uses a layered Win32 window and GDI presentation.
- macOS uses an accessory AppKit window and CALayer presentation.
- Linux uses X11, requires a compositor for transparency, and does not implement native Wayland overlays. XWayland availability alone does not guarantee an overlay above native Wayland applications.
- Drawing uses CPU rasterization with no GPU context. The embedded SKETCH arrow holds frames in the sequence `0,1,2,3,2,1` at 5 steps per second; the light target contour updates at about 12 fps. `animate: false` freezes both. The PNG atlas is decoded once when the first arrow is rendered; its four RGBA frames occupy about 0.68 MiB before other rendering buffers. Automatic local color sampling updates at approximately 1 Hz while visible. Static cues with a manual color or client palette redraw only as needed, but active window tracking still queries native window state. The event loop waits while hidden. Animation, tracking, and local color refresh have measurable CPU costs, so test them on the intended hardware.
- Protected desktops and some fullscreen applications may prevent a global overlay. Multi-monitor DPI changes and each OS/window-manager combination need desktop testing.
- CI builds Windows x64, macOS Apple Silicon, macOS Intel, and Linux x64/X11 binaries. Unit tests run natively on each Windows, macOS or Ubuntu runner; the Intel macOS build does not imply Intel GUI execution on an ARM runner. CI does not certify desktop interaction or compatibility with every MCP client. `tests/windows-native.ps1` tests one live Windows session without synthesizing clicks.

Code is MIT licensed. The embedded IBM Plex Mono font is distributed under [SIL OFL 1.1](../assets/OFL.txt). The generated SKETCH arrow atlas is embedded in the executable; the separate project mark is documentation artwork and is not loaded by the renderer.
