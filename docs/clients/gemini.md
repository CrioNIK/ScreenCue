# Gemini CLI

The ScreenCue extension bundles the native MCP server, a discoverable agent skill, short extension context, and `/screencue:show`, `/screencue:hide`, and `/screencue:status` commands. It connects directly from Gemini CLI to the local overlay without another model or relay.

This integration targets Gemini CLI. It does not add local MCP access to the Gemini website or mobile app.

## Install the extension

From the ScreenCue repository, build a package using the binary for your operating system:

```sh
python scripts/integrate.py bundle --client gemini --executable /absolute/path/to/screencue --output /absolute/path/to/screencue-bundle
gemini extensions validate /absolute/path/to/screencue-bundle/gemini
gemini extensions install /absolute/path/to/screencue-bundle/gemini
gemini extensions list
```

On Windows point to `screencue.exe` and quote paths containing spaces. Review the local extension prompt, then restart Gemini CLI. The extension references its bundled executable through the supported `${extensionPath}` variable. The installed extension is copied by Gemini CLI; use `gemini extensions update screencue` after updating its source package. See the official [extension reference](https://geminicli.com/docs/extensions/reference/).

The repository's manifest is a source template. The bundle command replaces `__SCREENCUE_BINARY__` and includes the executable and licenses. Installing the unbuilt repository subdirectory will not provide the binary.

## Configure an existing binary instead

```sh
python scripts/integrate.py configure --client gemini --executable /absolute/path/to/screencue --scope user
```

This merges the server into `~/.gemini/settings.json` and installs the skill under `~/.gemini/skills/screencue`. For a project use `--scope project --root /absolute/path/to/project`, producing `.gemini/settings.json` and `.gemini/skills/screencue`. The standalone route installs MCP and the skill; the extension additionally supplies the custom commands and its short context file. See [MCP configuration](https://geminicli.com/docs/tools/mcp-server/) and [agent skill discovery](https://geminicli.com/docs/cli/skills/).

Choose one route. A `settings.json` MCP entry with the same name overrides the extension server. If switching to the extension, remove only the old `mcpServers.screencue` entry and duplicate standalone skill.

## Use and check

In an interactive Gemini CLI session:

```text
/mcp
/skills list
/screencue:show the visible Settings button
/screencue:status
/screencue:hide
```

The skill may require Gemini CLI's normal activation confirmation. All four ScreenCue tools should be available: `screen_displays`, `screen_hint`, `screen_status`, and `screen_clear`.

Gemini CLI must separately have a permitted passive screen-observation capability to identify the requested control. ScreenCue cannot capture a screenshot or find UI elements by itself. Without observation, the agent reports that limitation rather than opening a file manager or changing windows. A schema or connection check alone does not prove visual pointing works.

The hint stays visible through clicks. It follows the original window locally, while scrolling and rearranged controls require a fresh target. Ctrl+Alt+X hides the cue by default (Ctrl+Option+X on macOS). The client must remain connected: ending its MCP process removes that process's hint.

For development, `gemini extensions link /absolute/path/to/screencue-bundle/gemini` uses the local package directly. Keep only one ScreenCue installation enabled. Running inside a remote machine or a display-less sandbox does not draw on the local desktop.
