---
name: screencue
description: Point out a visible desktop control or location with a ScreenCue arrow, guide one UI step, or clear an existing hint. Use when the user asks where to click or wants an on-screen pointer.
---

# ScreenCue

Call ScreenCue directly from the current agent. Keep the existing MCP connection alive; it owns its hint. Do not delegate pointing to another chat, task, or agent.

For a request to hide the cue, call `screen_clear` immediately. No screenshot or display lookup is needed.

For a pointing request:

1. Use a fresh, passive screen observation from the client's permitted screenshot or accessibility tools. ScreenCue does not see the screen or identify controls. Keep the current window in front: do not open a file manager, activate another app, minimize windows, or navigate elsewhere just to find the target. If passive observation is unavailable or the target is not visible, say so briefly instead of guessing.
2. Use `screen_displays` when physical monitor bounds are not already known for this display configuration. Convert the observation's coordinates using its documented capture origin and physical dimensions. ScreenCue uses physical desktop pixels, including negative monitor origins; browser CSS coordinates and resized-image pixels are not directly interchangeable.
3. As soon as the target is identified, call `screen_hint` once. Use an empty `text` or a few words. Let the user act, then give at most a short explanation. Do not add setup checks or repeated status polls before every hint.

Use `style: "arrow"` with `x` and `y` at the target center; `width` and `height` describe its physical bounds. For `style: "box"`, `x` and `y` are the top-left and width/height are required. Keep `color: "auto"`, the default bold sketch arrow, and `follow_window: true` for an application control. Use `follow_window: false` for desktop icons or other fixed screen positions. Arrow and target outline can be toggled independently with `arrow` and `target`.

Window movement is followed locally. Scrolling, resizing, and rearranged controls need a fresh observation and replacement hint. An offscreen or minimized window keeps an edge indicator. If a cue looks stale, inspect `screen_status` once; `tracking.state: "needs_refresh"` requires a new target.

Hints persist through clicks. Do not set `duration_ms` unless expiry is requested. Replace the hint for the next step; clear it when it is no longer relevant or the user asks. The default hide shortcut is Ctrl+Alt+X (Ctrl+Option+X on macOS); check `shortcut_active` and `shortcut_error` in the show result before promising it works.

The client supplies observation and permissions. This skill does not enable screenshots, computer control, or direct Dot access on a surface that lacks those capabilities.

## Conversation prompts

For a speaking note requested by the user, call `screen_prompt` with `text` and an optional `title` directly. No screenshot or display lookup is needed for a text prompt. Use the user's actual facts; do not invent experience, metrics, or commitments. ScreenCue does not automatically hear this client's conversation. Audio and model providers are configured separately in the desktop app. Capture privacy is controlled by the user; inspect `capture_privacy` and never promise invisibility to every screen-sharing tool. `screen_clear` hides active prompts and stops listening across connections sharing the settings file; the app's Hide all cues button and the hide shortcut notify active connections sharing its settings file.
