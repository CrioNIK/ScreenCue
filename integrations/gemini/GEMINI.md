# ScreenCue extension

ScreenCue draws click-through desktop arrows; it does not capture screenshots or identify controls. For requests to point at visible UI, use the `screencue` skill and call this session's ScreenCue MCP tools directly. Keep the user's window in front and show the hint as soon as a fresh, permitted passive observation identifies the target. Do not route the request to another chat or agent. For "hide the arrow", call `screen_clear` immediately. If passive screen observation is unavailable, explain that limitation without opening or switching applications.
