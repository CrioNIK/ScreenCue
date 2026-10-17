---
name: screencue
description: Point to a visible desktop control with a ScreenCue arrow, guide the next click, or hide an existing hint. Use when the user asks where something is on their current screen.
---

# ScreenCue

Use the connected ScreenCue MCP tools in this conversation. The plugin server may be named `plugin:screencue:screencue`; discover the tool names supplied by the client rather than inventing a prefix.

For a pointing request, observe the current screen with an already authorized passive screenshot or accessibility tool, then call `screen_hint` immediately. Use one short label, or `text: ""`. Explain briefly after the tool succeeds. Do not delegate to another agent or chat, open File Explorer, activate an app, switch windows, or click anything merely to find the target. ScreenCue itself cannot capture screens, identify controls, click, or type. If passive observation is unavailable or the target is not visible, state that limitation without moving the user's windows.

Map the fresh observation into physical desktop coordinates. Use `screen_displays` once if the monitor origin or scale is unknown; reuse it until the display configuration changes. For a capture resized from physical dimensions, apply its scale and origin to both coordinates and target size. Account for negative monitor origins. Never infer a coordinate from a filename, app name, stale screenshot, or generic UI layout.

Call `screen_hint` with the observed control center as `x` and `y`, its physical `width` and `height`, `style: "arrow"`, and `color: "auto"`. Leave `duration_ms: 0` and `follow_window: true` for application controls. The large red SKETCH arrow, separate contour, animation, and contrast adaptation are defaults. Use `follow_window: false` for fixed desktop icons. Keep the arrow and target enabled unless the user requests otherwise.

The native tracker follows window movement without model calls. It does not track scrolling or controls rearranged inside a window. Obtain a new observation and replace the hint after those changes or when `screen_status` reports `tracking.state: "needs_refresh"`. A minimized, closed, or offscreen target stays as a labelled edge cue; do not pretend it is a visible control. Avoid repeatedly polling status during ordinary pointing.

For "hide the arrow" or equivalent, call `screen_clear` immediately; no screenshot is needed. The default shortcut is **Ctrl+Alt+X**, or **Ctrl+Option+X** on macOS. Check the `screen_hint` result for `shortcut_active` and `shortcut_error` before claiming the shortcut is available. Do not clear a persistent hint merely because the assistant's turn ends. Each MCP process controls its own cue; clearing in another client does not clear this one.

If the tools are missing or a call fails, report the failure. Do not say an arrow is visible without a successful result.
