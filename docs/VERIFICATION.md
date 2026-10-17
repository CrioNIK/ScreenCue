# Verification record

Date: **2026-10-17**. Integration package version: **0.2.0**.

This record separates package validation, a real client's MCP connection, and an authenticated assistant showing a cue from a fresh screen observation. Passing the first two does not prove the third. None of these packages gives a client screen capture or UI recognition capabilities.

## Native server and integration tests

- All **44 Rust unit tests** passed, and a Windows release binary built successfully for 0.2.0.
- All **15 installer and package tests** passed. They cover generated configuration, preservation of unrelated settings, repeatable updates and backups, portable bundle contents, and failure handling in the MCP diagnostic path.
- The **0.2.0 Windows native suite** passed against the local GNU/LLVM release build. It checked click-through behavior, no focus theft, topmost recovery, persistent lifecycle, target movement, minimized/offscreen/restored/closed window states, shapes, color, shortcut conflicts and delivery, timed expiry, and EOF cleanup. One fixture observed window movement reflected in the cue in **52 ms**, including measurement overhead; this is a sample, not a latency guarantee.
- The MCP protocol smoke test passed on the binary extracted from the generated Claude Desktop MCPB. It covers initialization, tool discovery, notifications, argument validation, strict JSON, Unicode handling, and clean shutdown on EOF. This test does not draw a cue.
- The skill-creator validator accepted all **five skills**: Codex, Claude Code, Claude Desktop, Gemini, and Cursor.
- The Windows binary used for these package checks had SHA-256 `f0e50ef0f0972828f709603e710991273b4bf1065a08c25bc9c40067fc84bddc`. This identifies the local test build; release artifacts built by CI have their own published checksums.

## Client-specific evidence

| Integration | What was checked | Result | Still requires a client session |
| --- | --- | --- | --- |
| Codex | **0.163.0-alpha.10.2** installed and enabled the generated 0.2.0 plugin in an isolated environment. App-server `plugin/read` and `skills/list` discovered its server and skill. A temporary no-model thread used `mcpServerStatus/list` and `mcpServer/tool/call` with `screen_status`. | Plugin and skill discovery passed. All four MCP tools were connected; `screen_status` returned `ok: true`, version `0.2.0`, and `visible: false`. The temporary thread unsubscribed cleanly. | Fresh authenticated visual use of 0.2.0. Direct Dot/voice access depends on that runtime exposing these tools and was not established by this test. |
| Claude Code | Official CLI **2.1.302** ran `plugin validate --strict --json` against the generated 0.2.0 plugin. The same CLI ran `--plugin-dir <generated-plugin> mcp list` with an isolated configuration directory. | Validation passed without warnings. `plugin:screencue:screencue` reported **Connected** using the bundled binary. This exercised plugin discovery, `${CLAUDE_PLUGIN_ROOT}` expansion, process launch, and MCP initialization. | Signed-in model tool selection, passive observation, and a complete show/move/hide interaction. |
| Claude Desktop | Official **MCPB 2.1.2** validated the generated manifest, packed the rendered directory, and unpacked the project's `.mcpb` archive. The extracted executable passed the MCP protocol smoke test. | Manifest and archive accepted; native server protocol passed. | Installation in Desktop, connector discovery, skill upload, and authenticated visual use. Packages are unsigned. |
| Gemini CLI | **0.61.0** on Windows with **Node.js 24.12.0** validated the generated extension. An isolated extension link registered and enabled it; extension listing discovered its context, skill, and MCP server. | `skills list` and `mcp list` exited successfully; the bundled native server reported **Connected**. The link command printed success but then failed during CLI shutdown with a libuv `UV_HANDLE_CLOSING` assertion and exit code `3221226505`. This was reproduced with the normal launcher. | A clean extension-install lifecycle on that runtime, signed-in model tool selection, passive observation, and visual pointing. The connection result does not erase the shutdown failure. |
| Cursor | Cursor's official `plugin-template` validator ran against the generated 0.2.0 plugin in a temporary marketplace. | Validation passed. Its only warning was the absence of optional hooks; ScreenCue intentionally has no hooks. Manifest paths, skill frontmatter, rule frontmatter, and MCP-file presence were checked. | Cursor's own plugin loading, `${CURSOR_PLUGIN_ROOT}` expansion, trust flow, and authenticated visual use. |

The Claude CLI tests used a temporary `CLAUDE_CONFIG_DIR`. They did not sign into an account, call a model, modify the user's normal Claude configuration, or switch desktop windows. Package validation is not marketplace review or code signing.

Cursor validator source: [`cursor/plugin-template/scripts/validate-template.mjs`](https://github.com/cursor/plugin-template/blob/main/scripts/validate-template.mjs). The downloaded validator's SHA-256 was `826f55f546ce59500a6e3d7d32a15d90f3373cecc3b41486e75ae28b60647a4a`. Claude's supported validation command is described in its [plugin manifest reference](https://code.claude.com/docs/en/plugins-reference); MCPB validation follows its [manifest specification](https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md).

## Desktop behavior and platform boundaries

The new Windows 0.2.0 native suite tests ScreenCue against controlled window fixtures. The earlier 0.1.0 release was also used interactively from Codex on Windows. The fixture results and that earlier interactive session do not automatically establish a fresh authenticated visual test in every client.

macOS and Linux packages require testing in a graphical desktop session on their respective operating systems. Linux requires X11 with a compositor; native Wayland is unsupported. macOS requires 12.3 or later; local automatic color sampling additionally requires 15.2 or later and an already granted screen-recording permission. Claude Desktop packages are generated for Windows and macOS only.

Window movement is tracked locally. Scrolling, navigation, control rearrangement, and `tracking.state: "needs_refresh"` require the agent to obtain a new observation and replace the hint. A hidden or offscreen target produces a labelled edge cue. It is not evidence that the target is currently visible or clickable.

## Reproduce the visible test

1. Load exactly one ScreenCue connection in the client on the computer whose screen will be annotated.
2. Ask the same agent to call `screen_displays` directly. Verify the physical monitor bounds.
3. Ask it to observe a currently visible control passively and immediately call `screen_hint`. It must not activate another window, open File Explorer, click, or delegate the pointing request.
4. Move the target window. Confirm the cue follows without another model request. Minimize it and confirm the labelled edge cue remains.
5. Hide it with **Ctrl+Alt+X**, or **Ctrl+Option+X** on macOS. Then show another cue and ask the same agent to clear it with `screen_clear`.

If the client has no authorized passive screen observation, stop at that boundary and report it. Do not substitute guessed coordinates or a window-switching workflow for a successful test.
