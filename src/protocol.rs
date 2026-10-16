//! Dependency-light MCP stdio transport. The UI owns hint validation and rendering.
//! Nothing but newline-delimited JSON-RPC is written to stdout.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::sync::mpsc::{self, Sender};
use std::time::Duration;

const MAX_LINE_BYTES: usize = 32_768;
const REPLY_TIMEOUT: Duration = Duration::from_secs(5);
const LATEST_HANDSHAKE_VERSION: &str = "2025-11-25";

const INSTRUCTIONS: &str = "ScreenCue draws persistent click-through hints over the desktop without taking focus. \
It does not provide screenshots, identify controls, click, type, or control other apps. \
Auto color may sample a small local pixel region on show and about once per second while visible where the platform permits it; \
pixels are transient and are never saved or transmitted. Before placing a hint, obtain a fresh \
screen observation using your own permitted screen tools. Call screen_displays to inspect \
physical desktop bounds. Coordinates use physical desktop pixels, not browser CSS pixels. \
Map screenshot coordinates using that screenshot's captured desktop origin and physical \
dimensions; do not guess a scaling factor. Negative desktop coordinates are valid. \
Arrow and target outline are independent; target shapes include ellipse, rectangle, rounded \
rectangle, and polygon. For a pointing request, call screen_hint promptly for one step, \
with an empty label or a few words, and keep any accompanying explanation brief. \
Leave the current application in front and let the user click. Hints stay visible by default \
through clicks and window changes. Use screen_clear when the user finishes the step or the \
hint is no longer relevant; a new hint replaces it. Do not set duration_ms unless the user \
or task explicitly needs expiry. The configured hide shortcut also clears the hint. \
Closing the MCP connection ends its process and removes its hint. \
By default ScreenCue follows the native application window under the target locally, without another model call. \
This follows window movement and DPI, not scrolling or controls rearranging inside the window. \
After scrolling or UI layout changes obtain a fresh observation and replace the hint. \
Inspect tracking.state in screen_status: needs_refresh means a window resize invalidated the target. \
Offscreen, minimized and closed windows retain a labelled edge indicator. \
Set follow_window=false only for fixed desktop coordinates. \
Availability of global overlays and automatic color sampling depends on the operating system \
and display server; inspect status errors instead of assuming support.";

/// Sent to the GUI event loop. Reply exactly once with an object containing `ok`.
pub struct Command {
    pub request: Value,
    pub reply: Sender<Value>,
}

/// Run this blocking function on a worker thread; keep the UI event loop on the main thread.
/// Returning means stdin reached EOF, the peer closed stdout, or framing became unsafe.
pub fn serve(send: impl Fn(Command) -> Result<(), String> + Send + 'static) {
    let stdin = io::stdin();
    let stdout = io::stdout();
    if let Err(error) = serve_io(&mut stdin.lock(), &mut stdout.lock(), &send) {
        if error.kind() != io::ErrorKind::BrokenPipe {
            eprintln!("ScreenCue MCP: {error}");
        }
    }
}

fn serve_io<R: BufRead, W: Write, F: Fn(Command) -> Result<(), String>>(
    reader: &mut R,
    writer: &mut W,
    send: &F,
) -> io::Result<()> {
    let mut initialized = false;
    loop {
        let line = match read_line_bounded(reader)? {
            Line::Eof => return Ok(()),
            Line::TooLong => {
                write_message(
                    writer,
                    &error(
                        Value::Null,
                        -32700,
                        "Message exceeds the 32768-byte line limit.",
                    ),
                )?;
                // Never reinterpret an oversized message's remaining bytes as a new request.
                return Ok(());
            }
            Line::Text(line) => line,
        };
        if line.iter().all(|byte| matches!(byte, b' ' | b'\t' | b'\r')) {
            continue;
        }
        let message: Value = match serde_json::from_slice(&line) {
            Ok(value) => value,
            Err(_) => {
                write_message(
                    writer,
                    &error(
                        Value::Null,
                        -32700,
                        "Parse error: expected one JSON object per line.",
                    ),
                )?;
                continue;
            }
        };
        if let Some(response) = handle_message(message, &mut initialized, send, REPLY_TIMEOUT) {
            write_message(writer, &response)?;
        }
    }
}

fn write_message(writer: &mut impl Write, value: &Value) -> io::Result<()> {
    serde_json::to_writer(&mut *writer, value).map_err(io::Error::other)?;
    writer.write_all(b"\n")?;
    writer.flush()
}

enum Line {
    Eof,
    Text(Vec<u8>),
    TooLong,
}

fn read_line_bounded(reader: &mut impl BufRead) -> io::Result<Line> {
    let mut line = Vec::with_capacity(1024);
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            if line.is_empty() {
                return Ok(Line::Eof);
            }
            break;
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |position| position + 1);
        // Allow the newline and optional CR without increasing the content limit.
        if line.len() + count > MAX_LINE_BYTES + 2 {
            return Ok(Line::TooLong);
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            line.pop();
            break;
        }
        if line.len() > MAX_LINE_BYTES + 1 {
            return Ok(Line::TooLong);
        }
    }
    if line.last() == Some(&b'\r') {
        line.pop();
    }
    if line.len() > MAX_LINE_BYTES {
        Ok(Line::TooLong)
    } else {
        Ok(Line::Text(line))
    }
}

fn valid_id(id: &Value) -> bool {
    id.is_null() || id.is_string() || id.is_number()
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

fn result(id: Value, value: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": value})
}

fn tool_result(id: Value, reply: Value) -> Value {
    let is_error = reply.get("ok").and_then(Value::as_bool) != Some(true);
    result(
        id,
        json!({
            "content": [{"type": "text", "text": reply.to_string()}],
            "isError": is_error,
        }),
    )
}

fn handle_message<F: Fn(Command) -> Result<(), String>>(
    message: Value,
    initialized: &mut bool,
    send: &F,
    timeout: Duration,
) -> Option<Value> {
    let Some(object) = message.as_object() else {
        return Some(error(
            Value::Null,
            -32600,
            "Invalid JSON-RPC request: expected an object.",
        ));
    };
    let raw_id = object.get("id");
    let id = raw_id
        .filter(|id| valid_id(id))
        .cloned()
        .unwrap_or(Value::Null);
    let method = object.get("method").and_then(Value::as_str);
    if object.get("jsonrpc").and_then(Value::as_str) != Some("2.0")
        || method.is_none_or(str::is_empty)
        || raw_id.is_some_and(|id| !valid_id(id))
    {
        return Some(error(id, -32600, "Invalid JSON-RPC request."));
    }
    // This server issues no requests; every valid notification is ignored without a reply.
    if raw_id.is_none() {
        return None;
    }
    let empty = serde_json::Map::new();
    let parameters = match object.get("params") {
        None => &empty,
        Some(Value::Object(parameters)) => parameters,
        _ => return Some(error(id, -32602, "params must be an object.")),
    };
    match method.expect("validated method") {
        "initialize" => {
            let Some(requested) = parameters
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|value| !value.is_empty())
            else {
                return Some(error(
                    id,
                    -32602,
                    "initialize requires a protocolVersion string.",
                ));
            };
            if !parameters.get("clientInfo").is_some_and(Value::is_object)
                || !parameters.get("capabilities").is_some_and(Value::is_object)
            {
                return Some(error(
                    id,
                    -32602,
                    "initialize requires clientInfo and capabilities objects.",
                ));
            }
            let version = match requested {
                "2025-11-25" | "2025-06-18" | "2025-03-26" | "2024-11-05" => requested,
                _ => LATEST_HANDSHAKE_VERSION,
            };
            *initialized = true;
            Some(result(
                id,
                json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {"listChanged": false}},
                    "serverInfo": {"name": "screencue", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": INSTRUCTIONS,
                }),
            ))
        }
        "ping" => Some(result(id, json!({}))),
        "tools/list" | "tools/call" if !*initialized => Some(error(
            id,
            -32000,
            "Initialize this MCP connection before calling tools.",
        )),
        "tools/list" => Some(result(id, json!({"tools": tool_definitions()}))),
        "tools/call" => {
            let name = parameters.get("name").and_then(Value::as_str);
            let command = match name {
                Some("screen_hint") => "show",
                Some("screen_clear") => "clear",
                Some("screen_status") => "status",
                Some("screen_displays") => "displays",
                _ => return Some(error(id, -32602, "Unknown or missing tool name.")),
            };
            let mut arguments = match parameters.get("arguments") {
                None => serde_json::Map::new(),
                Some(Value::Object(arguments)) => arguments.clone(),
                _ => return Some(error(id, -32602, "arguments must be an object.")),
            };
            if command != "show" && !arguments.is_empty() {
                return Some(error(id, -32602, "This tool does not accept arguments."));
            }
            if arguments.contains_key("command") {
                return Some(error(id, -32602, "command is not a tool argument."));
            }
            arguments.insert("command".to_owned(), Value::String(command.to_owned()));
            let (reply, receive) = mpsc::channel();
            let request = Command {
                request: Value::Object(arguments),
                reply,
            };
            let response = match send(request) {
                Err(message) => json!({"ok": false, "error": message}),
                Ok(()) => match receive.recv_timeout(timeout) {
                    Ok(response) if response.get("ok").is_some_and(Value::is_boolean) => response,
                    Ok(_) => {
                        json!({"ok": false, "error": "The overlay returned an invalid response."})
                    }
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        json!({"ok": false, "error": "The overlay did not respond within 5 seconds."})
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => {
                        json!({"ok": false, "error": "The overlay stopped before replying."})
                    }
                },
            };
            Some(tool_result(id, response))
        }
        _ => Some(error(id, -32601, "Method not found.")),
    }
}

/// Tool schema is public so frontends and development tooling can inspect it without a GUI.
pub fn tool_definitions() -> Value {
    let coordinate = json!({"type": "integer", "minimum": i32::MIN, "maximum": i32::MAX});
    let empty_schema = json!({"type": "object", "properties": {}, "additionalProperties": false});
    let query_annotations = json!({"readOnlyHint": true, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false});
    json!([
        {
            "name": "screen_hint",
            "description": "Draw one click-through desktop hint without taking focus, replacing the previous hint. Returns promptly; the cue stays visible by default through clicks and window changes until screen_clear, replacement, the hide shortcut, or process shutdown. Prefer an empty label or a few words, then let the user act. Obtain a fresh screen observation first and map screenshot coordinates to physical desktop pixels using its capture origin and scale. This tool does not provide screenshots, identify controls, or click. Auto color may sample local pixels about once per second while visible, without saving or transmitting them. Box style requires width and height; polygon shape requires points and an interior x/y anchor. Check shortcut_active and shortcut_error in the result.",
            "inputSchema": {
                "type": "object",
                "properties": {
                    "x": {"type": "integer", "minimum": i32::MIN, "maximum": i32::MAX, "description": "Physical desktop x: target center for arrow style, top-left for box style, or interior anchor for polygon. May be negative."},
                    "y": {"type": "integer", "minimum": i32::MIN, "maximum": i32::MAX, "description": "Physical desktop y: target center for arrow style, top-left for box style, or interior anchor for polygon. May be negative."},
                    "text": {"type": "string", "maxLength": 500, "default": "Click here", "description": "Short instruction beside the target. Empty string hides the text card."},
                    "animate": {"type":"boolean","default":true,"description":"The four-frame SKETCH arrow advances at 5 steps per second; the light target contour animates at about 12 frames per second. False freezes both."},
                    "follow_window": {"type":"boolean","default":true,"description":"Locally follow the original app window under the target. Movement is tracked without another agent call; offscreen/minimized targets keep a labelled screen-edge cue. Does not follow scrolling or rearranged controls: refresh the hint after UI changes. False fixes desktop coordinates."},
                    "background_colors": {"type":"array","maxItems":16,"items":{"type":"string","pattern":"^#[0-9A-Fa-f]{6}$"},"description":"Optional representative colors from a fresh screenshot around the target, including repeated common colors. Auto color uses this palette without local pixel sampling; useful on macOS."},
                    "style": {"type": "string", "enum": ["arrow", "box"], "default": "arrow"},
                    "color": {"type": "string", "pattern": "^(auto|#[0-9A-Fa-f]{6})$", "minLength": 4, "maxLength": 7, "default": "auto", "description": "auto samples a small local pixel region about once per second while visible where supported, without saving or transmitting pixels. Or provide exact #RRGGBB."},
                    "shape": {"type": "string", "enum": ["ellipse", "rectangle", "rounded", "polygon"], "description": "Independent target shape. Default ellipse for arrow style, rectangle for box style. Polygon requires points and x/y inside it."},
                    "arrow": {"type": "boolean", "description": "Show the arrow independently. Default true for arrow style, false for box style."},
                    "arrow_scale": {"type": "number", "minimum": 0.75, "maximum": 2.5, "default": 1.6, "description": "Arrow length and swept head size, independent of target size. Default 1.6 gives a larger, bolder curved arrow; the target outline stays lighter and its separation gap stays unchanged. Length may be constrained by available screen space."},
                    "target": {"type": "boolean", "default": true, "description": "Show the target outline independently of the arrow."},
                    "points": {"type": "array", "minItems": 3, "maxItems": 64, "items": {"type": "object", "properties": {"x": coordinate, "y": coordinate}, "required": ["x", "y"], "additionalProperties": false}, "description": "Polygon vertices in boundary order, in absolute physical desktop pixels. Requires shape polygon; the x/y anchor must lie inside."},
                    "width": {"type": "integer", "minimum": 1, "maximum": 4000, "description": "Target width in physical pixels. Required for box style; default 16 for arrow style."},
                    "height": {"type": "integer", "minimum": 1, "maximum": 4000, "description": "Target height in physical pixels. Required for box style; default 16 for arrow style."},
                    "duration_ms": {"type": "integer", "default": 0, "anyOf": [{"const": 0}, {"minimum": 500, "maximum": 300000}], "description": "Default 0: stays until the agent clears/replaces it, the user presses the hide shortcut, or the process exits. Clicks and window changes do not dismiss it. Set 500..300000 only when timed expiry is explicitly needed."},
                    "step": {"type": "integer", "minimum": 1, "maximum": 99, "description": "Optional numbered instruction badge."}
                },
                "required": ["x", "y"],
                "additionalProperties": false
            },
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": false, "openWorldHint": false}
        },
        {
            "name": "screen_clear", "description": "Remove the current desktop hint.",
            "inputSchema": empty_schema,
            "annotations": {"readOnlyHint": false, "destructiveHint": false, "idempotentHint": true, "openWorldHint": false}
        },
        {
            "name": "screen_status", "description": "Inspect the overlay status, platform limitations, and current hint. Does not provide a screenshot.",
            "inputSchema": empty_schema, "annotations": query_annotations
        },
        {
            "name": "screen_displays", "description": "Get monitor bounds in physical desktop pixels, including negative origins. Use a fresh screenshot's capture origin and dimensions to map target coordinates. Does not provide a screenshot.",
            "inputSchema": empty_schema, "annotations": query_annotations
        }
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::sync::{Arc, Mutex};

    fn no_dispatch(_: Command) -> Result<(), String> {
        panic!("protocol-only request reached renderer");
    }

    #[test]
    fn strict_json_and_rpc_errors_are_distinct() {
        let input =
            b"{'id':1}\n{\"a\":1,}\n[]\nnull\n{\"jsonrpc\":\"2.0\",\"method\":\"ping\",\"id\":7}\n";
        let mut output = Vec::new();
        serve_io(&mut Cursor::new(input), &mut output, &no_dispatch).unwrap();
        let replies: Vec<Value> = String::from_utf8(output)
            .unwrap()
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect();
        assert_eq!(replies[0]["error"]["code"], -32700);
        assert_eq!(replies[1]["error"]["code"], -32700);
        assert_eq!(replies[2]["error"]["code"], -32600);
        assert_eq!(replies[3]["error"]["code"], -32600);
        assert_eq!(replies[4]["result"], json!({}));
    }

    #[test]
    fn oversized_line_stops_without_parsing_the_tail() {
        let input = format!(
            "{}\n{{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}}\n",
            " ".repeat(MAX_LINE_BYTES + 1)
        );
        let mut output = Vec::new();
        serve_io(&mut Cursor::new(input), &mut output, &no_dispatch).unwrap();
        let output = String::from_utf8(output).unwrap();
        assert_eq!(output.lines().count(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(output.trim()).unwrap()["error"]["code"],
            -32700
        );
    }

    #[test]
    fn crlf_boundary_is_counted_without_line_endings() {
        let mut input = Cursor::new(format!("{}\r\n", " ".repeat(MAX_LINE_BYTES)));
        assert!(
            matches!(read_line_bounded(&mut input).unwrap(), Line::Text(line) if line.len() == MAX_LINE_BYTES)
        );
    }

    #[test]
    fn notification_has_no_response_and_unicode_id_is_preserved() {
        let mut initialized = false;
        assert!(handle_message(
            json!({"jsonrpc":"2.0","method":"notifications/unknown"}),
            &mut initialized,
            &no_dispatch,
            REPLY_TIMEOUT
        )
        .is_none());
        let response = handle_message(
            json!({"jsonrpc":"2.0","id":"запрос-世界","method":"ping"}),
            &mut initialized,
            &no_dispatch,
            REPLY_TIMEOUT,
        )
        .unwrap();
        assert_eq!(response["id"], "запрос-世界");
    }

    #[test]
    fn tool_dispatch_keeps_hint_fields_and_reports_failures() {
        let observed = Arc::new(Mutex::new(Value::Null));
        let observed_in_send = observed.clone();
        let send = move |command: Command| {
            *observed_in_send.lock().unwrap() = command.request;
            command
                .reply
                .send(json!({"ok":true,"visible":true}))
                .unwrap();
            Ok(())
        };
        let request = json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"screen_hint","arguments":{"x":1,"y":2,"text":"世界 😀","arrow":false,"arrow_scale":1.6,"color":"#aBc123"}}});
        let response = handle_message(request.clone(), &mut true, &send, REPLY_TIMEOUT).unwrap();
        assert_eq!(response["result"]["isError"], false);
        let forwarded = observed.lock().unwrap();
        assert_eq!(forwarded["command"], "show");
        assert_eq!(forwarded["text"], "世界 😀");
        assert_eq!(forwarded["arrow"], false);
        assert_eq!(forwarded["arrow_scale"], 1.6);
        assert_eq!(forwarded["color"], "#aBc123");
        let response = handle_message(
            request,
            &mut true,
            &|_| Err("UI unavailable".to_owned()),
            REPLY_TIMEOUT,
        )
        .unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("UI unavailable"));
    }

    #[test]
    fn arrow_scale_schema_matches_model_range() {
        let definitions = tool_definitions();
        let schema = &definitions[0]["inputSchema"]["properties"]["arrow_scale"];
        assert_eq!(schema["type"], "number");
        assert_eq!(schema["minimum"], 0.75);
        assert_eq!(schema["maximum"], 2.5);
        assert_eq!(schema["default"], 1.6);
    }

    #[test]
    fn duration_schema_matches_persistent_model_default() {
        let definitions = tool_definitions();
        let schema = &definitions[0]["inputSchema"]["properties"]["duration_ms"];
        let hint = crate::model::Hint::parse(json!({"x":100,"y":100})).unwrap();
        assert_eq!(schema["default"], hint.duration_ms);
        assert_eq!(schema["default"], 0);
        assert_eq!(schema["anyOf"][0]["const"], 0);
        assert_eq!(schema["anyOf"][1]["minimum"], 500);
        assert_eq!(schema["anyOf"][1]["maximum"], 300000);
    }

    #[test]
    fn missing_renderer_reply_is_bounded() {
        let held_reply = Mutex::new(None);
        let send = |command: Command| {
            *held_reply.lock().unwrap() = Some(command.reply);
            Ok(())
        };
        let request =
            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"screen_status"}});
        let response = handle_message(request, &mut true, &send, Duration::from_millis(1)).unwrap();
        assert_eq!(response["result"]["isError"], true);
        assert!(response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("did not respond"));
    }
}
