# ScreenCue 0.0.4 verification

## Scope

This release adds the native settings app and application icon, independent conversation prompts, Windows capture exclusion, an optional native audio helper, three independent conversation-inference lanes, and desktop installation packages. All checks below distinguish implementation and offline fixtures from live model and meeting compatibility.

## Local Windows checks

- Native Rust renderer/settings suite: 68 tests passed.
- Native audio-helper Rust suite: 12 tests passed.
- Python integration, controller, conversation, desktop-process and packaging suites: 84 tests passed with the built native binaries enabled.
- MCP lifecycle and live hints passed, including `screen_prompt`, text-only geometry, malformed input, replacement preservation, and clean EOF.
- Native overlay suite passed: click-through styles, no focus change, persistent hints, local window tracking, minimize/offscreen/restore behavior, animation, hide shortcut, and shutdown.
- Settings-window test passed: native icon, visibility, minimize, restore and close; capture policy exactly `0x11` when enabled and `0x0` when disabled on both settings and overlay windows; no capture or model service started.
- Five independent desktop-process tests passed: shared settings and dismissal across connections, settings loading before first display, conversation defaults and explicit arrow overrides, and Start/Stop/restart through owned UI controls. A synthetic helper emitted a deliberately late hint and deliberately ignored shutdown; the app suppressed that hint and reaped the helper.
- Native helper process tests use synthetic transcripts, local fake HTTP responses and explicit command fixtures. They cover provider validation before capture, speaker/role fields, independently progressing transcription and suggestions, stale generations, Stop, EOF and command cancellation.
- The 23 conversation orchestrator tests use bounded fake providers and a local SSE fixture. They cover independent lanes, partial/final revisions, role routing, stale output, prepared candidates, stream cancellation, fresh observed screen geometry, and lane-local prompt budgets that preserve the current turn.

No real microphone or system audio was captured during verification. No account credentials, paid model request, actual Jev service or live meeting application was used. Speaker-diarization accuracy and speech-to-first-useful-cue latency remain unmeasured. Native capture-policy readback is not a test of another application's recording or the receiver's view.

## Observed native footprint

The local GNU Windows renderer was approximately 2.9 MiB and the native helper 0.88 MiB before packaging. These sizes differ from MSVC release builds and exclude optional Python/model runtimes.

A three-second sample on the development PC observed 0 CPU milliseconds while hidden, about 14 MiB working set while hidden, and about 30 MiB with a text panel. A fixed static hint used 0 sampled CPU milliseconds; animated tracking plus adaptive color used about 4.7% of one core. Short samples and scheduler granularity are not performance guarantees. Conversation/model workloads were not included.

## Build and installer coverage

CI defines Windows x64, macOS Apple Silicon, macOS Intel and Linux x64/X11 builds. It checks both Rust crates, Python suites, native Windows UI/privacy, package contents, Windows installer install/uninstall, Linux user installation/removal, and macOS bundle signatures/disk-image integrity.

Local packaging tests validate explicit public inputs, required licenses, executable bits, metadata and exclusion of private configuration. The macOS layout check stubs signing, disk-image generation and symlink creation; native macOS CI performs those operations and verifies the result. Python resources live under `Contents/Resources`, outside the signed native executables directory.

The local Inno Setup bootstrap was blocked by automatic execution review; it was not retried locally. Windows installer compilation and lifecycle verification use the clean CI runner. Final CI results and downloadable artifacts are linked from the release.

## Remaining boundaries

- Windows capture exclusion only applies to compatible capture paths. macOS and Linux report it unsupported.
- Native Wayland, arbitrary exclusive-fullscreen games, and hardware capture are not universally supported.
- Short ASR chunks do not establish a persistent human identity. Use a persistent diarization adapter for stable speaker IDs; map roles explicitly.
- Fast/slow/decision adapters require explicit configuration. The optional advanced orchestrator also requires an existing Python 3.11+ interpreter.
- The app and installer are unsigned; macOS uses ad-hoc integrity signing, not notarization.
- Custom native settings controls support keyboard input but do not yet expose full screen-reader semantics.
