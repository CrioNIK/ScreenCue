# Contributing to ScreenCue

Help make native AI guidance reliable on real desktops. Useful contributions include a small Rust fix, a reproducible desktop test, clearer client setup instructions, and Ukrainian documentation. You do not need to work on every part of the project.

English is the primary language for shared documentation, issues, and pull requests. Ukrainian contributions and discussion are welcome. Keep command names, API fields, and executable examples unchanged when translating.

## Pick a small task

Start with [good first issues](https://github.com/CrioNIK/ScreenCue/labels/good%20first%20issue) or [help wanted](https://github.com/CrioNIK/ScreenCue/labels/help%20wanted). If a task interests you, comment with the part you would like to tackle and check for an existing pull request. For a new feature or a broad change, open an issue with the problem, proposed scope, and a way to verify it before starting a large implementation. Small documentation corrections can go straight to a pull request.

Especially useful starting points:

- Reproduce and document behavior on one named macOS or Linux X11 desktop.
- Improve one client guide after following it from a clean setup.
- Document keyboard navigation or report a specific accessibility obstacle.
- Add a focused regression test for a reproducible bug.

## Find the right part of the repository

| Area | Start here | Boundary |
| --- | --- | --- |
| Native renderer and settings app | [`src/`](src/), [`Cargo.toml`](Cargo.toml), [integration reference](docs/INTEGRATION.md) | Draws supplied hints and panels; does not identify controls, provide screenshots to agents, click, or type. |
| Optional native audio helper | [`conversation/`](conversation/), [helper guide](conversation/README.md) | Separate Rust binary for explicitly started audio sessions and configured transcription/suggestion adapters. |
| Optional Python controllers | [`controller/`](controller/), [decision controller](controller/README.md), [conversation orchestration](controller/CONVERSATION.md) | Uses supplied observations or transcripts and explicitly configured providers; separate from the renderer. |
| Client integrations | [`integrations/`](integrations/), [`scripts/`](scripts/), [client guides](docs/clients/) | Client-specific packages, configuration helpers, and agent guidance. |
| Packaging and checks | [`packaging/`](packaging/), [`tests/`](tests/), [CI workflow](.github/workflows/ci.yml) | Native packages, automated checks, and platform-specific test entry points. |

The renderer's lack of model/network requests does not apply to optional helpers configured to use hosted providers. Keep these components and their requirements distinct in code and documentation.

## Set up from source

Fork the repository, clone your fork, and create a branch for one change. Work from current `main`; source may be ahead of the downloadable release. Record the commit when reporting results:

```sh
git rev-parse HEAD
```

Install stable Rust with `rustfmt`. Standard Windows builds need C++ Build Tools; macOS needs Command Line Tools. On Linux, follow the dependency list in the [CI workflow](.github/workflows/ci.yml), including X11/XKB/XCB libraries and ALSA/OpenSSL development libraries when building the audio helper. Native Wayland is not implemented; overlay checks need X11 with a compositor.

From the repository root, build the renderer/settings executable:

```sh
cargo build --release --locked
```

It is written to `target/release/screencue` (`screencue.exe` on Windows). For a graphical smoke check, run that executable with `demo`; hide the cue with Ctrl+Alt+X, or Ctrl+Option+X on macOS.

Build the separate audio helper only when needed for your work:

```sh
cargo build --release --locked --manifest-path conversation/Cargo.toml
```

Its output is under `conversation/target/release/`. The helper requires macOS 14.2 or newer; the standalone renderer has a lower deployment target. See [installation](docs/INSTALL.md) for platform and packaging requirements, and the [helper guide](conversation/README.md) for audio-specific limits.

Python 3.11+ is needed for the optional controllers and Python development checks; CI currently uses Python 3.13. Node.js is needed for the MCP smoke test. These are development or optional-component requirements, not requirements to run the native renderer.

## Verify the change

Run the checks relevant to what you changed. For a documentation-only change, check paths, examples, and the accuracy of claims; a full native build is not required. Use the [CI workflow](.github/workflows/ci.yml) as the current reference for the complete build and package checks.

For renderer/settings Rust changes:

```sh
cargo fmt --check
cargo test --locked
cargo build --release --locked
```

For audio-helper Rust changes:

```sh
cargo fmt --manifest-path conversation/Cargo.toml --check
cargo test --manifest-path conversation/Cargo.toml --locked
cargo build --release --manifest-path conversation/Cargo.toml --locked
python -m unittest discover -s tests -p 'test_conversation.py' -v
```

The helper process tests find a local debug/release build, or use `SCREENCUE_LISTEN_BINARY` set to its absolute path. They skip if no helper binary exists; report skips as skips. They use synthetic transcripts and local fixtures, not live microphones or model accounts.

For integration scripts, controllers, or packaging changes:

```sh
python -m unittest discover -s tests -p 'test_*.py' -v
```

Native process tests have additional binary/desktop requirements. In particular, `test_desktop_app.py` is opt-in through `SCREENCUE_DESKTOP_BINARY`; ordinary discovery skips it. A passing offline suite does not verify a live AI client, audio provider, or desktop session.

For MCP changes, run the protocol smoke test against your built renderer:

```sh
node tests/mcp-smoke.cjs target/release/screencue
```

Use `target/release/screencue.exe` on Windows. Add `--live` in a graphical session to exercise visible hints.

For Windows overlay or settings changes, run the relevant native check in your own graphical session:

```powershell
powershell -NoProfile -ExecutionPolicy Bypass -File .\tests\windows-native.ps1
powershell -NoProfile -ExecutionPolicy Bypass -File .\tests\windows-app.ps1
```

These checks own their test windows and processes. The overlay suite's automatic-color phase reads transient screen pixels. They do not test real microphones or model services.

## Share reproducible desktop results

For a bug report or native validation, include the commit/release, operating system and architecture, display scale and monitor layout, and the client version if relevant. On Linux, also name the X11 session and compositor. Give exact steps, expected and observed behavior, and mark each check as passed, failed, or not tested.

Useful overlay checks include click-through behavior, focus retention, show/clear, the hide shortcut, moving/minimizing/restoring the target window, and text-panel layout. Include a small screenshot or recording only when it helps explain a visible issue and contains no private content. Keep credentials and personal configuration out of reports and commits.

Build success does not establish native desktop behavior. Separate offline fixtures from live testing, and document the exact environment tested. Do not generalize one successful screen-sharing test to all capture applications, or one fullscreen application to all games. See the [verification report](docs/VERIFICATION-0.0.4.md), [capture privacy](docs/CAPTURE-PRIVACY.md), and [fullscreen limits](docs/FULLSCREEN-PLAN.md).

## Open a focused pull request

Keep one problem per pull request and avoid unrelated formatting or dependency changes. Link the issue if there is one, explain the user-visible result, and list the relevant checks with their results and any skips or remaining gaps. Include before/after evidence for visible changes. Add a regression test when it demonstrates the bug being fixed, and update the affected documentation when behavior changes.

Check the diff for generated binaries, personal paths, provider credentials, and private transcripts before pushing. Do not commit personal client settings. Keep new assets and dependencies compatible with the project's [MIT license](LICENSE), and document their provenance and required notices.
