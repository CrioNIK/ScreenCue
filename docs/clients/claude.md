# Claude Code and Claude Desktop

ScreenCue runs on the same computer as the visible desktop. Choose **one** connection per client to avoid two overlays competing for the hide shortcut. The native binary uses stdio directly; no model proxy, HTTP listener, API key, or JavaScript runtime is added.

## Claude Code: configure an existing installation

From the ScreenCue checkout, run:

```sh
python scripts/integrate.py configure --client claude-code --scope project --root /absolute/path/to/your-project --executable /absolute/path/to/screencue
```

On Windows, use the absolute path to `screencue.exe`, quoted if it contains spaces. The installer merges the ScreenCue MCP entry and copies the skill, backing up files it changes. Use `--scope user` for your local account instead of a single project. Restart the Claude Code session and check `/mcp`. Accept the client's normal trust prompt for this local server.

The skill guides short, direct pointing requests. With standalone skill installation, invoke `/screencue` or ask naturally. ScreenCue tools may have a client-specific prefix; use the tools Claude actually lists.

## Claude Code: portable plugin

Generate a plugin with its native binary inside:

```sh
python scripts/integrate.py bundle --client claude-code --executable /absolute/path/to/screencue --platform linux-x64-x11 --output ./dist/integrations
claude plugin validate ./dist/integrations/claude-code
claude --plugin-dir ./dist/integrations/claude-code
```

Choose `windows-x64`, `macos-apple-silicon`, `macos-intel`, or `linux-x64-x11` to match the supplied binary. The generated plugin resolves its executable with `${CLAUDE_PLUGIN_ROOT}`, so it remains usable after moving the whole folder. The source template under `integrations/claude-code` must be rendered before use.

In the plugin session, `/mcp` should show `plugin:screencue:screencue`, and `/screencue:screencue` invokes the skill. `--plugin-dir` loads the plugin for that session; it does not alter permanent client settings. Claude's [plugin creation guide](https://code.claude.com/docs/en/plugins/create) explains local loading. The package follows the [manifest reference](https://code.claude.com/docs/en/plugins-reference) and [MCP component format](https://code.claude.com/docs/en/plugins/components).

## Claude Desktop: native extension

Generate the platform-specific extension:

```sh
python scripts/integrate.py bundle --client claude-desktop --executable /absolute/path/to/screencue --platform macos-apple-silicon --output ./dist/integrations
```

In Claude Desktop, open **Settings > Extensions > Advanced settings > Install Extension** and choose the generated `.mcpb` file. Confirm ScreenCue appears under connected tools. This is a native binary extension, distinct from the Claude Code plugin. See Claude's [local extension instructions](https://support.claude.com/en/articles/10949351-getting-started-with-local-mcp-servers-on-claude-desktop) and the [MCPB manifest specification](https://github.com/modelcontextprotocol/mcpb/blob/main/MANIFEST.md). Organization policy may restrict custom extensions.

The extension starts the MCP server but does not auto-install a custom skill. Upload the separately generated skill ZIP under **Customize > Skills > + Create skill > Upload a skill**, if your account permits custom skills. The ZIP contains `screencue/SKILL.md`; it supplies guidance and does not connect the local server itself. See [Claude custom skills](https://support.claude.com/en/articles/12512180-use-skills-in-claude).

For clients configured with the legacy JSON file, use this alternative instead of installing the extension:

```sh
python scripts/integrate.py configure --client claude-desktop --scope user --executable /absolute/path/to/screencue
```

Restart Desktop after changing its configuration. Do not enable both the JSON connection and the extension.

## Verify the connection and the visible behavior

After using `configure`, run the repository's `doctor` command for the same client and scope. It reads that configuration and checks the executable with an MCP handshake, tool discovery, and read-only desktop status. This checks the native server, not Claude's account or UI integration.

```sh
python scripts/integrate.py doctor --client claude-code --scope project --root /absolute/path/to/your-project
```

Inside Claude, ask: **"Call ScreenCue screen_displays directly, without delegating or switching windows."** Check that its tool call returns the actual monitor bounds. Then ask it to point to a control already visible on your screen using its authorized passive observation tool. Drag that control's window, hide it with **Ctrl+Alt+X** (**Ctrl+Option+X** on macOS), and ask **"Clear the ScreenCue hint."** The last request should directly call `screen_clear`.

ScreenCue does not supply screen vision. A client without passive screen observation cannot locate controls automatically. A skill upload cannot grant that missing capability. Cloud-only sessions do not gain access to your desktop from these local packages. The same-session show, move, and clear checks are required before calling a specific Claude environment fully verified.
