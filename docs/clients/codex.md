# Codex

ScreenCue connects directly to local Codex through MCP. The package includes the native server, a discoverable `screencue` skill, portable Agent Plugins metadata, and a Codex compatibility manifest. It adds no model API, relay service, or second chat.

## Configure an existing binary

Run from the ScreenCue repository, replacing the executable path with your downloaded binary:

```sh
python scripts/integrate.py configure --client codex --executable /absolute/path/to/screencue --scope user
```

On Windows use an absolute path to `screencue.exe`, quoted if it contains spaces. For a project installation, use `--scope project --root /absolute/path/to/project`. The installer merges the MCP entry and copies the skill. Codex reads user MCP settings from `~/.codex/config.toml`, project settings from `.codex/config.toml`, and skills from `.agents/skills`. Project configuration requires a trusted project. See [Codex MCP configuration](https://learn.chatgpt.com/docs/extend/mcp?surface=cli) and [skill discovery](https://learn.chatgpt.com/docs/build-skills).

For a manual MCP-only connection:

```sh
codex mcp add screencue -- /absolute/path/to/screencue --mcp
codex mcp get screencue
```

Copy `integrations/codex/skills/screencue` into `~/.agents/skills/` to add the workflow. Start a new session after setup.

## Install the bundled plugin

Build a package for the binary's platform:

```sh
python scripts/integrate.py bundle --client codex --executable /absolute/path/to/screencue --output /absolute/path/to/screencue-bundle
codex plugin marketplace add /absolute/path/to/screencue-bundle
codex plugin add screencue@screencue-local
```

The bundle contains `codex/bin/screencue` (or `screencue.exe`) and its licenses. Its marketplace at `.agents/plugins/marketplace.json` points to `./codex`, relative to the bundle root. Current Codex CLI supports `plugin add`; if your version does not, open the local marketplace in the desktop Plugins Directory and install ScreenCue there. See [local marketplace packaging](https://developers.openai.com/plugins/build/plugins).

Choose either the plugin or the standalone configuration. Enabling both creates separate server processes and duplicate tools. A cue belongs to the process that created it; clearing a different process does not clear that cue.

The tracked integration files are source templates. The bundle command substitutes `__SCREENCUE_BINARY__` and includes the executable; copying the source directory alone does not create a runnable plugin. The portable `mcp.json` uses the contained relative command `./bin/screencue` (or `.exe`), which Codex resolves from the plugin root. The compatibility `.mcp.json` uses `${PLUGIN_ROOT}`. A root variable in the portable command field is rejected by Codex 0.163, even though variables are available in other local-server fields; see the [local MCP server guide](https://developers.openai.com/plugins/deploy/desktop-only). Public GitHub distribution is separate from acceptance into OpenAI's public plugin directory.

## Use and check

Ask: `Use $screencue to show me where to click on my current screen.`

The agent needs its own permitted passive screen-observation tool. ScreenCue never opens or switches applications to find a target. If the client has no screen access, it can still run `screen_displays` and `screen_status`, but cannot identify an unseen control.

Check that all four tools appear: `screen_displays`, `screen_hint`, `screen_status`, and `screen_clear`. Show one hint on an observed target, move its window, then ask to clear it. Ctrl+Alt+X hides it by default; macOS uses Ctrl+Option+X. Observe any shortcut-registration error returned by the server.

## Dot and remote sessions

This package targets local Codex execution on the desktop displaying the overlay. It does not establish that a Dot voice session can call arbitrary local MCP tools directly. A connected computer or a working Codex installation is not proof of Dot availability. Direct Dot invocation remains unverified; do not silently route it through another chat. Cloud or remote execution draws on the execution machine, not automatically on the user's local screen.
