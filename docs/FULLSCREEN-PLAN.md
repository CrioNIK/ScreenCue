# Fullscreen overlays: support and implementation plan

Research checked on October 17, 2026. This document distinguishes the current implementation from proposed backends. It is not a claim that every game, graphics API, compositor, or display mode has been tested.

ScreenCue should support fullscreen through the presentation facilities of each operating system. An always-on-top desktop window alone cannot guarantee visibility over every exclusive fullscreen game. Model choice does not change that limitation.

## Current implementation

| Environment | Existing path | What remains unverified |
| --- | --- | --- |
| Windows desktop | Per-pixel-alpha Win32 layered window; click-through, no activation; native foreground and stacking notifications restore topmost order while visible | A representative game/display-mode matrix, HDR, VRR, multiple GPUs |
| Windows borderless fullscreen | The same desktop window and stacking observer | Visibility and frame-time effects in actual games; independent flip and MPO behavior depend on the system |
| Windows true exclusive fullscreen | No dedicated in-game presentation backend | Desktop visibility is not guaranteed; repeated raising cannot establish support |
| macOS | Accessory application; layer-backed AppKit window at status level; ignores mouse input; `canJoinAllSpaces` and `fullScreenAuxiliary`; non-key ordering | Other applications' fullscreen Spaces, Stage Manager, and Metal games on real Macs |
| Linux/X11 | ARGB override-redirect window; input passes through; compositor required; event-driven restacking | Fullscreen unredirection behavior across window managers and games |
| Native Wayland | No backend in the current build | Layer-shell presentation, input behavior, capture, and target tracking |

Evidence for the existing paths is in `src/platform.rs` and window creation in `src/main.rs`. The current `visible: true` status means ScreenCue mapped its native window; it does **not** prove the user can see it over a game. Native window following also does not track a moving object inside a game's rendered scene.

## Windows: preserve the lightweight desktop path

Windows Fullscreen Optimizations can run a game that requests fullscreen in an optimized borderless presentation mode. Microsoft describes this as allowing desktop composition to resume when an overlay appears. Therefore a game's menu label, or a screen-sized window rectangle, is insufficient evidence of true exclusive presentation. The latter gives the game control of the display; Microsoft describes interception of the render/present path as the traditional way external overlays enter it. ScreenCue does not implement that path. [Microsoft: Fullscreen Optimizations](https://devblogs.microsoft.com/directx/demystifying-full-screen-optimizations/)

DirectFlip, independent flip, and multi-plane overlays are distinct from a universal ban on desktop overlays. When another surface appears, Windows can resume composition, use reverse composition, or retain independent flip through MPO. This makes the current native window a reasonable first backend for borderless games, subject to measurement. Hardware overlay planes are compositor/driver decisions, not a generic layer ScreenCue can reserve above every game. [Microsoft: DXGI flip model](https://learn.microsoft.com/en-us/windows/win32/direct3ddxgi/for-best-performance--use-dxgi-flip-model)

Proposed next work:

1. Keep the current Win32 path as the default. Avoid constant topmost polling, a full-monitor transparent raster, or system-wide changes to MPO, drivers, or game settings.
2. Presentation diagnostics now expose `backend`, `window_mapped`, `fullscreen_visibility: unverified`, and `exclusive_fullscreen_guaranteed: false`. These report native window state, not a game's DXGI swap-chain mode.
3. Build an optional Game Bar companion that receives the same small hint scene as the desktop renderer. The SDK renders a UWP XAML view within Game Bar's own overlay space, so this is a separate packaged host, not another Win32 window flag. Keep it out of the minimal standalone binary. [Microsoft: Game Bar architecture](https://learn.microsoft.com/en-us/xbox/game-bar/overview)
4. Support pinning and the user's click-through setting. Pinning keeps a widget available when Game Bar is dismissed; click-through lets input reach the underlying game. Test whether widget bounds, transparency, resize limits, and multi-monitor placement can meet ScreenCue's free-positioned arrow design before promising parity. [Widget manifest](https://learn.microsoft.com/en-us/gaming/game-bar/guide/pkg-manifest), [click-through](https://learn.microsoft.com/en-us/xbox/game-bar/guide/click-through)

Game Bar is a supported integration route to investigate, not proof of compatibility with every exclusive game. Where a game provides an official overlay or mod API, an explicit adapter can render the same scene inside that game. Otherwise report that presentation is unsupported and offer borderless mode or another display. Do not silently change the user's display mode, inject into game processes, or disable game protection.

## macOS: prototype a proper nonactivating panel

The current status-level `NSWindow` and Spaces flags are not enough to claim fullscreen support. Apple's `canJoinAllApplications` behavior specifically addresses joining other applications in fullscreen and Stage Manager, subject to eligibility; it is mutually exclusive with the `primary` and `auxiliary` Stage Manager behaviors. [Apple: collection behavior](https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct/canjoinallapplications)

Apple DTS published a tested macOS 26 recipe using accessory activation, an `NSPanel` with the nonactivating style, Spaces/fullscreen collection behaviors, and screen-saver window level. The engineer tested Safari, QuickTime Player, and fullscreen YouTube. That is evidence for a prototype, not for all Metal games or ScreenCue's supported older macOS releases. [Apple DTS: fullscreen overlay panel](https://developer.apple.com/forums/thread/826308)

ScreenCue already uses accessory activation. Prototype the remaining panel and collection-behavior changes as an isolated AppKit presentation option with runtime availability checks, rather than changing only the numeric window level. Preserve click-through and non-key behavior. Test activation, Spaces transitions, multiple displays, hide/show, and older macOS before promoting it. Hide during session locking; elevated presentation level is not a reason to cover protected system UI.

## Linux: X11 hardening and a separate Wayland backend

The X11 presenter now sets `_NET_WM_BYPASS_COMPOSITOR=2` and `_NET_WM_WINDOW_TYPE_NOTIFICATION` on **ScreenCue's own overlay** before mapping. This expresses that the overlay requires composition and lets compositors classify the override-redirect surface; it neither changes another application's property nor forces every compositor to redirect a fullscreen game. EWMH explicitly defines both hints and leaves compositor policy in control. Runtime behavior on Linux desktops still needs verification. [freedesktop.org: application window properties](https://specifications.freedesktop.org/wm/latest/ar01s05.html)

A Wayland implementation must negotiate a supported protocol rather than reuse the X11 path. A layer-shell surface on the `overlay` layer, keyboard interactivity `none`, and an empty input region is the appropriate prototype. Fullscreen shell surfaces are normally below that layer; compositor behavior still needs testing. Sway's own overlay bar uses an empty input region, demonstrating the click-through mechanism. [Layer-shell protocol source](https://github.com/swaywm/wlr-protocols/blob/master/unstable/wlr-layer-shell-unstable-v1.xml), [current Sway implementation](https://github.com/swaywm/sway/blob/master/swaybar/bar.c)

Discover `zwlr_layer_shell_v1` at runtime and return an explicit unsupported capability if absent. Layer-shell implementations currently document support on wlroots, Smithay, Mir, and KDE Plasma, but not GNOME Wayland. A GNOME Shell extension would be a separate integration with its own lifecycle and verification. Do not advertise XWayland as universal access to native Wayland fullscreen surfaces. [GTK4 Layer Shell: supported desktops](https://github.com/wmww/gtk4-layer-shell#supported-desktops)

Wayland presentation support does not automatically provide global window coordinates, screen capture permission, or a global hotkey. Those require separate compositor/portal capabilities. The renderer should expose what is available before an agent attempts a hint.

## Fast reactions and target stability

Fullscreen support must preserve these independent responsibilities:

- **Presentation:** a small already-running native renderer receives a scene and displays it. Show a minimal confirmed cue first; add richer text without waiting for a complete explanation.
- **Window anchoring:** move an existing cue with its saved native window identity. The current 16 ms Windows / 33 ms macOS-X11 polling intervals are scheduling targets, not end-to-end latency guarantees. Native geometry events can reduce idle polling; keep bounded reconciliation for missed events.
- **Object anchoring:** use accessibility identity for application controls where available; otherwise use a local visual tracker over a small region. Re-identify only after loss or layout change. A window rectangle cannot follow a moving in-game object.
- **Freshness:** carry observation time, display scale, anchor identity, confidence, and a monotonically increasing revision. Reject late model results after the target or scene changes. A lost target becomes an edge/lost-target indicator, never a precise arrow to stale pixels.
- **Input focus:** presentation never activates the cue. Keep the current hide shortcut and agent clear command independent of model inference and animations.

These are implementation priorities, not measured performance claims. Keep capture, local inference, and remote inference optional so users who already have an agent do not pay for a second always-running model.

## Acceptance checks before publishing fullscreen support

| Test group | Required observations |
| --- | --- |
| Windows display modes | Windowed, borderless, and verified exclusive; D3D11, D3D12, Vulkan/OpenGL where available; overlay appears without minimizing or deactivating the game |
| Display configurations | Main/secondary monitor, negative desktop coordinates, mixed DPI, HDR/SDR, VRR, resolution changes |
| macOS | Intel and Apple Silicon where supported; fullscreen Spaces; Stage Manager; nonactivating panel behavior; Metal game |
| Linux | At least two X11 compositors with fullscreen unredirection; native Wayland layer-shell compositors; missing-protocol fallback |
| Interaction | Clicks and keyboard remain with the application; hide works immediately; clear and disconnect remove every surface |
| Tracking | Window movement, resizing, minimization, scene/object motion, target loss, stale responses, monitor removal |
| Performance | Command-to-first-visible-frame and tracking error at p50/p95; game frame times with cue hidden, static, and animated; CPU/GPU and memory separately |

Record OS, GPU/driver, game/version, actual presentation mode, and backend for each result. A build passing CI or a mapped-window API response does not substitute for visual fullscreen validation.
