# Verification: 0.0.3

Date: October 17, 2026. This prototype uses the revised [versioning policy](VERSIONING.md); the lower number does not represent a feature rollback from the earlier 0.2.0 release.

## Local checks

- **56 Rust tests passed**, including independent panels, complete pagination of long text, fractional display scales, opaque text backgrounds, whitespace without missing-glyph boxes, panel/arrow separation, color precedence, and animation cache reuse.
- **43 Python tests passed**: 15 integration/package tests and 28 optional-controller tests. The run treated resource warnings as errors. Provider tests use fake loopback endpoints and local command fixtures, not live model services.
- The release binary built successfully and reported `0.0.3`. Rust formatting and Git whitespace checks passed.
- The **live MCP smoke test passed**, including title-only panels, independent panel position, long-text pages, schema validation, and lifecycle checks.
- The **Windows native suite passed**: click-through behavior, no focus theft, topmost recovery, window following and edge states, shortcut ownership/delivery, expiry, invalid-replacement preservation, long-text pagination, and EOF cleanup. Its controlled window-movement sample was **38 ms**, not a latency guarantee.
- An additional controller-to-native JSON smoke test passed: a supplied test candidate produced a text-only panel on page 2 of 6, then controller clear removed it. No model, game, or screen-recognition service was involved.
- The actual renderer's [panel previews](panels-preview.png) were visually checked on light and dark backgrounds. This caught and fixed control-character boxes before completion. These are rendered fixtures, not screenshots of a user's applications.

The local Windows GNU/LLVM executable was **2,771,456 bytes (2.64 MiB)**, SHA-256 `f2994374768eb370780b25a406b1db193169d5d6c6dea3086f122825872ddc4e`. CI-produced artifacts have separate checksums.

## Resource samples

The native suite took approximately two-second samples on this Windows machine. The hidden instance used 13.91 MiB working set; visible cases after loading text assets used 29.87–30.56 MiB. Hidden and static samples recorded zero CPU timer ticks; animated cases recorded approximately 0.78–2.33% of one logical core. Short timer-granularity samples do not establish zero CPU use or universal performance guarantees. No-text rendering does not load the Sans fonts or panel texture.

Earlier show-latency measurements isolated the color/render-pass optimization using the old compact card; see [LATENCY.md](LATENCY.md). They are not new-panel, model, network, or display-scanout measurements.

## Boundaries

This work does not establish live Jev/OpenAI/Ollama accuracy or latency, automatic screen perception, tracking of controls inside a scrolling window, direct Dot voice-tool access, or authenticated use in every client. Native hide does not yet propagate cancellation to the optional controller; controller clear does invalidate pending inference. The earlier [client verification record](VERIFICATION.md) remains the source for actual client versions tested.

No specific game was tested, as requested. True exclusive fullscreen, macOS fullscreen behavior, and Linux compositor behavior remain unverified. Game Bar, native Wayland, and in-game rendering backends are not implemented. The X11 composition hints and explicit presentation diagnostics improve capability reporting without asserting universal visibility. See [FULLSCREEN-PLAN.md](FULLSCREEN-PLAN.md).
