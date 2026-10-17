# Latency and target continuity

ScreenCue has two separate latency budgets: finding the right target, and drawing an already known target. A faster renderer cannot remove time spent waiting for a screenshot, an agent, or a remote model. Keep those measurements separate.

## Implemented rendering improvement

The show path now resolves a supplied color or background palette before the first raster. It reuses that raster when the selected color is unchanged, including local auto-color samples that retain the primary red.

Previously every show rasterized twice. A non-red cue also retinted the sketch cache to red and back on every request. Removing both operations preserves the same pixels, geometry, validation, sampling rules, and focus behavior. Local auto-color still needs a second raster when the sampled background calls for a different color.

Two regression tests cover supplied-color precedence and contrast, and compare every output pixel and arrow landmark against the previous two-pass path at ordinary and screen-edge positions.

### Local measurement before the new panel design, 2026-10-17

These measurements isolate the color/render-pass change using the previous compact card. They are not measurements of the later brush panel, pagination, or optional decision controller.

Windows GNU/LLVM release builds used Rust 1.99.1 and the same Cargo settings. The baseline binary was the existing 0.2.0 local release. Four runs used the order before, after, after, before; each case had two initial requests followed by 30 measured requests through one persistent JSON process. The cue included an arrow, target, and `Open settings` card. Animation and window following were disabled to isolate show latency. No other application window was activated or moved.

| Supplied color | Before median, two runs | After median, two runs |
| --- | --- | --- |
| Primary red | 10.43–10.63 ms | 8.47–8.48 ms |
| Cyan | 24.86–25.42 ms | 8.46–8.55 ms |
| Auto with a client palette | 24.73–25.29 ms | 8.05–8.32 ms |

The non-red cases improved by about 66–68% in this fixture; primary red improved by about 19–20%. The first show after process initialization remained around 22–24 ms. These are local samples, not performance guarantees. The measured interval includes JSON transport and the return from native presentation; it excludes model inference, network latency, screenshot acquisition, and physical display scanout. This does not measure fullscreen game compatibility.

All 46 Rust tests and the Windows native regression suite passed. The native suite verified click-through behavior, focus preservation, topmost recovery, shapes, colors, failed-replacement preservation, tracking and edge indicators, shortcut ownership, expiry, and process cleanup. Its owned-window movement fixture observed an update in 13 ms on this run; timing depends on scheduling and is not a guaranteed response time.

## Existing local behavior

- Window following polls native geometry every 16 ms on Windows and 33 ms on macOS/X11 while a target is tracked. These are scheduling intervals, not end-to-end latency promises.
- Ordinary movement repositions the native overlay and its cached pixels. It rerenders when the target state, display, scale, or available layout changes.
- Window movement and DPI changes are computed from the original anchor, preventing accumulated translation error. Offscreen, minimized, and closed targets use labelled edge indicators.
- A resize reports `needs_refresh`. Window anchoring does not recognize scrolling, rearranged controls, or moving objects inside a game.
- Clear and the hide shortcut stop visible animation, tracking, and background sampling. Hidden instances wait for events.
- The overlay is click-through and avoids activating its native window. Tracking a target and changing the user's keyboard focus are separate operations.

## Next improvements

1. Keep the overlay process and agent transport warm. Send a cue as soon as a target is sufficiently certain; do not wait for explanatory prose. Clear directly without a new screenshot or reasoning pass.
2. Give each observation a sequence, capture timestamp, coordinate transform, and target identity. Reject delayed results from an older observation or a superseded request before they can replace the current cue. Clearing must invalidate pending results too.
3. Let local tracking maintain a confirmed target between model calls. Native window movement needs no inference. Accessibility identifiers or short-lived visual tracking can handle movement inside the window, with confidence checks and an explicit refresh when confidence falls.
4. Use bounded queues and keep only the newest unprocessed observation. Trigger expensive recognition when the scene changes or the tracker loses the target, rather than repeatedly uploading the whole desktop.

The optional [decision controller](../controller/README.md) now implements observation revisions, deadlines, stale-answer rejection, and a bounded pending queue. These controls are separate from the renderer optimization above; the core renderer does not identify UI controls. Accessibility identity and local visual tracking inside a window remain future work.
