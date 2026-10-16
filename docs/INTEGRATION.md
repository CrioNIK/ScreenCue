# Integrating ScreenCue

ScreenCue is a local desktop renderer with two stdin/stdout interfaces. It provides no model, screen-understanding service, screenshot tool, or clicking API. An agent observes the target application separately, then sends coordinates for one temporary cue.

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

The server implements MCP revisions `2025-11-25`, `2025-06-18`, `2025-03-26`, and `2024-11-05`; an unsupported initialization revision receives `2025-11-25` for negotiation. Its framing follows the [MCP stdio transport specification](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports): UTF-8 JSON-RPC messages, one message per line, protocol output only on stdout. Diagnostics use stderr. Maximum message content is 32,768 bytes.

## Tools

| Tool | Purpose |
| --- | --- |
| `screen_displays` | Return physical monitor origins, dimensions, and scale factors |
| `screen_hint` | Replace this process's cue |
| `screen_clear` | Hide this process's cue |
| `screen_status` | Return visibility, current hint, process ID, and color source |

Each running `--mcp` process owns its own overlay. Closing its stdin ends the process and removes the overlay. There is no shared daemon, socket, tray, or global hotkey. Separate agents may launch separate instances; coordinate ownership in your client if you want only one cue on screen.

### Hint arguments

| Field | Meaning |
| --- | --- |
| `x`, `y` | Required integer coordinates in physical desktop pixels |
| `text` | Up to 500 characters; `""` hides the text card; default `"Click here"` |
| `style` | `"arrow"` by default; `"box"` uses a top-left anchor and requires `width`/`height` |
| `shape` | `"ellipse"`, `"rectangle"`, `"rounded"`, or `"polygon"`; defaults to ellipse for arrow style and rectangle for box style |
| `width`, `height` | Outline dimensions, 1–4,000 pixels; default 16 × 16 for arrow style |
| `arrow` | Boolean; defaults to true for arrow style and false for box style |
| `target` | Boolean controlling the outline independently of the arrow; defaults to true |
| `points` | For polygon: 3–64 absolute desktop points, each `{ "x": integer, "y": integer }` |
| `color` | `"auto"` or an exact `"#RRGGBB"` value |
| `background_colors` | Optional array of up to 16 `"#RRGGBB"` colors observed by the agent; used by automatic color selection before attempting local sampling |
| `duration_ms` | 500–300,000 milliseconds; default 15,000; 0 persists until replaced, cleared, or process exit |
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

With `auto`, a nonempty `background_colors` palette takes priority over local sampling. An agent can provide colors from its fresh observation, including on macOS:

```json
{"x":600,"y":400,"text":"Open settings","color":"auto","background_colors":["#FAFAFA","#202124","#E0E0E0"]}
```

Without a supplied palette, `auto` samples a local region up to 128 × 128 pixels once per cue on Windows/X11. Samples are transient and never saved or transmitted. A successful response includes the selected `color`, `color_source` (`client_palette`, `local_pixels`, `manual`, or `fallback`), and an optional `color_note`. macOS currently uses a fallback when no client palette is supplied. An explicit hex color avoids sampling altogether. The CLI equivalent is `--background-colors "#FAFAFA,#202124"`.

## Coordinates and observation

Call `screen_displays` before mapping a screenshot. Coordinates refer to the physical virtual desktop and can be negative for monitors to the left of or above the primary monitor. They are not browser CSS coordinates or macOS logical points.

For a screenshot of a known physical desktop rectangle, map an image point `(u,v)` as:

```text
x = desktop_left + u * captured_physical_width  / image_width
y = desktop_top  + v * captured_physical_height / image_height
```

Round to integer pixels. For a crop, include its physical origin. Obtain capture dimensions and scale from the actual observation tool; do not assume that the monitor scale factor alone describes a resized screenshot. Refresh coordinates after scrolling, moving a window, changing zoom, or changing displays.

Suggested agent instruction:

> Explain one step at a time. Use a fresh, authorized observation to locate the control. Read ScreenCue's display geometry and map the target to physical desktop pixels. Show a brief cue, let the user act, and clear stale cues. Never infer that showing a cue means the action was completed.

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
screencue show --x 600 --y 400 --text "Open settings" --duration-ms 8000
screencue show --x 600 --y 400 --shape polygon --points "550,350;670,360;650,450;560,440" --text "Choose this area"
```

Each `show` command starts its own process, prints its result, and stays alive until expiry. Duration 0 means it stays alive until terminated. A separate CLI invocation cannot clear a different process's cue; use MCP/JSON `clear` within the connection that created it.

## Platform notes and testing

- Windows uses a layered Win32 window and GDI presentation.
- macOS uses an accessory AppKit window and CALayer presentation.
- Linux uses X11, requires a compositor for transparency, and does not implement native Wayland overlays. XWayland availability alone does not guarantee an overlay above native Wayland applications.
- Drawing uses CPU rasterization, with no GPU context or continuous redraw loop. The event loop waits for commands, window events, or cue expiry.
- Protected desktops and some fullscreen applications may prevent a global overlay. Multi-monitor DPI changes and each OS/window-manager combination need desktop testing.
- CI compiles and runs unit tests on Windows, macOS and Ubuntu; it does not certify desktop interaction or compatibility with every MCP client. `tests/windows-native.ps1` tests one live Windows session without synthesizing clicks.

Code is MIT licensed. The embedded IBM Plex Mono font is distributed under [SIL OFL 1.1](../assets/OFL.txt). The generated project mark is documentation artwork and is not loaded by the renderer.
