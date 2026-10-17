---
name: screencue
description: Point to a visible desktop control with a ScreenCue arrow, guide the next click, or hide an existing hint. Use when the user asks where something is on their current screen.
---

# ScreenCue

Use the connected ScreenCue MCP tools in this conversation. Discover the tool names supplied by the client rather than inventing a prefix. The skill does not connect the server by itself.

For a pointing request, observe the current screen with an already authorized passive screenshot or accessibility tool, then call `screen_hint` immediately. Use one short label, or `text: ""`. Explain briefly after the tool succeeds. Do not delegate to another agent or chat, open File Explorer, activate an app, switch windows, or click anything merely to find the target. ScreenCue itself cannot capture screens, identify controls, click, or type. If passive observation is unavailable or the target is not visible, state that limitation without moving the user's windows.

Map the fresh observation into physical desktop coordinates. Use `screen_displays` once if the monitor origin or scale is unknown; reuse it until the display configuration changes. For a capture resized from physical dimensions, apply its scale and origin to both coordinates and target size. Account for negative monitor origins. Never infer a coordinate from a filename, app name, stale screenshot, or generic UI layout.

Call `screen_hint` with the observed control center as `x` and `y`, its physical `width` and `height`, `style: "arrow"`, and `color: "auto"`. Leave `duration_ms: 0` and `follow_window: true` for application controls. The large red SKETCH arrow, separate contour, animation, and contrast adaptation are defaults. Use `follow_window: false` for fixed desktop icons. Keep the arrow and target enabled unless the user requests otherwise.

The native tracker follows window movement without model calls. It does not track scrolling or controls rearranged inside a window. Obtain a new observation and replace the hint after those changes or when `screen_status` reports `tracking.state: "needs_refresh"`. A minimized, closed, or offscreen target stays as a labelled edge cue; do not pretend it is a visible control. Avoid repeatedly polling status during ordinary pointing.

For "hide the arrow" or equivalent, call `screen_clear` immediately; no screenshot is needed. The default shortcut is **Ctrl+Alt+X**, or **Ctrl+Option+X** on macOS. Check the `screen_hint` result for `shortcut_active` and `shortcut_error` before claiming the shortcut is available. Do not clear a persistent hint merely because the assistant's turn ends. Each MCP process draws its own cue; clear and the hide shortcut also notify active connections sharing the same settings file.

If the tools are missing or a call fails, report the failure. Do not say an arrow is visible without a successful result.

## Conversation prompts

For a speaking note requested by the user, call `screen_prompt` with `text` and an optional `title` directly. No screenshot or display lookup is needed for a text prompt. Use the user's actual facts; do not invent experience, metrics, or commitments. ScreenCue does not automatically hear this client's conversation. Audio and model providers are configured separately in the desktop app. Capture privacy is controlled by the user; inspect `capture_privacy` and never promise invisibility to every screen-sharing tool. `screen_clear` hides active prompts and stops listening across connections sharing the settings file; the app's Hide all cues button and the hide shortcut notify active connections sharing its settings file.
