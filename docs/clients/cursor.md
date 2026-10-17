# Cursor

Use ScreenCue with a **local Cursor agent on the computer whose desktop you want to annotate**. The MCP server launches the native binary directly. Remote and cloud agents do not gain access to your local display by installing a skill.

## Configure MCP and the skill

From the ScreenCue checkout:

```sh
python scripts/integrate.py configure --client cursor --scope project --root /absolute/path/to/your-project --executable /absolute/path/to/screencue
```

On Windows, supply the absolute path to `screencue.exe`, quoted if needed. This adds the MCP entry to `.cursor/mcp.json`, the skill to `.cursor/skills/screencue/SKILL.md`, and the scoped rule to `.cursor/rules/screencue.mdc`. Existing unrelated settings remain intact; changed files are backed up. Choose `--scope user` to configure `~/.cursor/mcp.json` and the user skill instead. Global Cursor rules are managed in Cursor's UI; the installer does not invent a user rules directory.

Open Cursor's **Customize > MCP** view and enable or reload the ScreenCue server as required. The skill should appear in **Customize > Skills** and can be invoked with `/screencue`. Follow the client's normal tool approval flow. The configuration uses Cursor's documented [MCP support](https://cursor.com/docs/mcp), [skill locations](https://cursor.com/docs/skills), and [project rules](https://cursor.com/docs/rules).

## Portable Cursor plugin

Build a package containing the binary, MCP definition, skill, and rule:

```sh
python scripts/integrate.py bundle --client cursor --executable /absolute/path/to/screencue --platform linux-x64-x11 --output ./dist/integrations
```

Use the platform matching the binary: `windows-x64`, `macos-apple-silicon`, `macos-intel`, or `linux-x64-x11`. Copy the generated `cursor` directory into `~/.cursor/plugins/local/screencue`. Restart Cursor or run **Developer: Reload Window**, then check the plugin and its MCP tools in Customize. This is the [documented local plugin workflow](https://cursor.com/docs/plugins), not a claim of publication in Cursor Marketplace. Organization policy can disable local imports.

The generated `mcp.json` uses `${CURSOR_PLUGIN_ROOT}/bin/...` as its command. Cursor expands this in plugin MCP configuration; the generic `${PLUGIN_ROOT}` token is not supported there. The package follows Cursor's [native plugin reference](https://cursor.com/docs/reference/plugins). The repository template must be rendered by the bundler before use. Install either this plugin or the standalone MCP/skill configuration, not both.

## Check it in your session

Ask **"Call ScreenCue screen_displays directly; do not delegate or switch windows."** Verify that the result describes your actual monitors. Ask for an arrow at a control that is already visible, using an authorized passive screen observation tool. The same agent should call `screen_hint` promptly, without launching File Explorer or changing focus.

Move the target window and confirm that the cue follows. Ask **"Hide the arrow"** and confirm a direct `screen_clear` call. **Ctrl+Alt+X** hides the cue independently of the agent, or **Ctrl+Option+X** on macOS. If the tool reports a shortcut conflict, choose another shortcut when starting ScreenCue.

ScreenCue has no screenshot or click tool. A browser-only observation is usable only when its viewport origin and scale can be mapped to physical desktop coordinates. If no passive observation is available, the agent must say so instead of guessing a target or switching windows. Window tracking does not follow scrolling or rearranged controls; those require a fresh observation and hint.

After standalone configuration, run `python scripts/integrate.py doctor --client cursor --scope project --root /absolute/path/to/your-project` to test the configured MCP server. The repository's integration tests and `doctor` command verify configuration, packaging, and the MCP server. They do not substitute for the above authenticated Cursor session test.
