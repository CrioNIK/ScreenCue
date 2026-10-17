# Conversation hints

`screencue-listen` is an optional native helper for ScreenCue's conversation mode. It captures requested audio sources, sends short in-memory WAV segments to the configured transcription adapter, and asks a separately configured model or agent for concise speaking notes. The ScreenCue app starts and stops this helper and displays its `hint` events as independent text panels.

No model, model weights, Python runtime, account, API key, or recording service is bundled. Nothing is captured until the user starts a configured session. Stopping closes the capture streams before the helper acknowledges `stopped`; queued and in-flight answers cannot emit another hint afterward. The app also discards events from a previous session.

## Configure and start

The app can create and open a configuration file. The command-line equivalents are:

```sh
screencue-listen --write-example conversation.json
screencue-listen --validate-config conversation.json
screencue-listen --config conversation.json --start
```

Creating an example never overwrites an existing file. The example requests both microphone and system audio, but leaves both providers unset. It intentionally fails validation until a transcription provider is configured. Validation does not open audio devices, read API keys, run commands, or contact providers. The following explicit configuration uses already-running local services:

```json
{
  "capture": {
    "microphone": true,
    "system_audio": true,
    "microphone_device": null,
    "system_audio_device": null,
    "chunk_ms": 2400,
    "silence_ms": 350,
    "silence_threshold": 0.006
  },
  "transcription": {
    "kind": "openai-compatible",
    "endpoint": "http://127.0.0.1:8080/v1/audio/transcriptions",
    "model": "your-installed-transcription-model",
    "key_env": "",
    "timeout_ms": 8000
  },
  "suggestion": {
    "kind": "openai-compatible",
    "endpoint": "http://127.0.0.1:11434/v1/chat/completions",
    "model": "your-installed-chat-model",
    "key_env": "",
    "timeout_ms": 8000
  },
  "context": "My company builds accessibility tools. We have three pilot customers. Help me explain our progress clearly and ask useful follow-up questions."
}
```

Use the full URL for each operation. Transcription requires multipart `model` and `file` support and a JSON `{"text":"..."}` response. Suggestions require the Chat Completions `messages` request and `choices[0].message.content` response. Compatible local servers and remote services can be mixed. Provider compatibility depends on the actual server, model and endpoint; no live provider integration is implied by offline tests.

Only HTTPS and loopback HTTP endpoints are accepted. Redirects are disabled. URL credentials and query strings are rejected. Put a credential's **environment-variable name** in `key_env`; keep its value outside the configuration file. Providers inherit the environment of the ScreenCue app. An empty `key_env` sends no Authorization header. Endpoint names are user choices, and a local service can itself forward requests elsewhere. Native services are not installed or started automatically.

`suggestion: null` produces transcript events only. With both capture booleans false, the helper accepts external transcript commands without opening a microphone or output-device stream. An agent can also send a finished hint directly to ScreenCue over MCP; that path needs neither the helper nor a model round trip.

## Models and agents through commands

Either provider can be a local executable or wrapper around any model or agent:

```json
{
  "kind": "command",
  "command": ["/absolute/path/to/adapter", "--your-option"],
  "timeout_ms": 8000
}
```

No shell is used. The wrapper receives one UTF-8 JSON object on stdin, followed by EOF. For transcription:

```json
{"version":1,"operation":"transcribe","source":"system_audio","mime_type":"audio/wav","audio_base64":"...","timeout_ms":8000}
```

For suggestions:

```json
{"version":1,"operation":"suggest","context":"User-supplied facts and instructions","transcript":"system_audio: What is your traction?","timeout_ms":8000}
```

Return exactly `{"text":"..."}` on stdout. A suggestion adapter can return `{"text":"","refused":true}` to decline. Logs belong on stderr, which this helper discards. Wrappers must clean up their own descendants and close inherited pipes. ScreenCue terminates the direct child on timeout or stop; it does not control independently spawned background services. Keep heavyweight local inference in an already-running server so launching the wrapper stays inexpensive.

## App transport

Keep stdin open for the lifetime of the session. Send `{"command":"stop"}` or close stdin to stop. In external-transcript mode, send:

```json
{"command":"transcript","text":"What would you spend the investment on?"}
```

Stdout is JSON Lines:

```json
{"event":"status","state":"listening","microphone":true,"system_audio":true}
{"event":"transcript","source":"system_audio","text":"What is your traction?"}
{"event":"hint","title":"Suggested words","text":"We have three pilot customers. The next milestone is validating repeat usage."}
{"event":"status","state":"stopped"}
```

Errors use `{"event":"error","stage":"transcription","message":"..."}`. The app must treat provider output and transcripts as text, never instructions or commands. It owns panel placement, privacy settings, the hide shortcut and session cancellation. A hint contains no coordinates, executable content or renderer command.

Source labels distinguish the microphone, system output and externally supplied text; they are not speaker diarization. Microphone plus loudspeaker playback can capture the same speech twice. Headphones reduce acoustic duplication; this helper does not implement echo cancellation.

## Speaker labels and roles

The helper preserves speaker labels provided by a transcription model or an external diarization adapter. A compatible transcription provider can set `"response_format":"diarized_json"` to request `segments[].speaker`. Command adapters can return `{"segments":[{"speaker_id":"speaker-0","text":"A question"},{"speaker_id":"speaker-1","text":"An answer"}]}`. External transcript commands accept the same segments, or a single `text` and `speaker_id`, with an optional `source` of `microphone`, `system_audio` or `external`.

Map known labels explicitly in the configuration:

```json
{
  "speaker_roles": {
    "external:speaker-0": "investor",
    "external:speaker-1": "presenter"
  },
  "source_roles": {
    "microphone": "presenter"
  }
}
```

Source-qualified speaker mappings take precedence over bare speaker IDs, then source mappings. No role or identity is inferred when a mapping is absent. Transcript events include `speaker_id` and `speaker_role`, either strings or null. The role appears in the quick model's bounded conversation history.

Speaker labels from independent short audio requests may change between requests. This helper does not maintain voice embeddings or infer stable human identity from those labels. Use a persistent diarization adapter with stable speaker IDs when role continuity matters; mapping an unstable `A`/`B` label is not identity verification. The built-in chunking and diarization request shape have offline tests, but actual multi-speaker accuracy has not been measured.

The native `suggestion` provider is an optional quick response stage. Set it to null when a separate orchestrator consumes transcript events for independent fast suggestions, slower planning, and typed decision routing. Those stages need not be serialized behind this helper's quick response model. Merely configuring a source does not provide persistent diarization or a planning system.

## Audio support and limits

| Platform | Microphone | System audio |
| --- | --- | --- |
| Windows | Native WASAPI input | Native WASAPI output-device loopback |
| macOS | CoreAudio input; helper requires macOS 14.2+ | CPAL CoreAudio loopback requires macOS 14.6+ and applicable system permission |
| Linux | Default CPAL/ALSA input | Configure `system_audio_device` to an available monitor/loopback input; automatic desktop routing is not implemented |

Optional device selectors use CPAL device IDs returned by `screencue-listen --list-devices`; listing does not open capture streams. A `system_audio_device` selects an explicit input instead of native output-device loopback. Requested streams must all open successfully; an unavailable source causes a visible failure rather than silently falling back to the other source. Device changes/errors stop the session so the app can ask the user to start again after correcting the device. Protected or exclusive output and desktop permission policy can limit capture. No microphone or system-audio capture was activated in the automated test suite.

The callback downmixes PCM channels to mono and performs lightweight amplitude-based segmentation. A pause ends a segment after 350 ms by default; continuous speech is capped at 2.4 seconds. This is bounded chunk transcription, not a streaming speech-recognition protocol or a semantic voice-activity detector. Recognition and suggestion accuracy depend on the chosen models. Segment boundaries may split words. Audio remains in memory, and neither audio nor transcripts are written to disk by the helper. Selected external providers and adapters have their own data handling.

Transcription and quick suggestions have independent workers. Transcript events continue while a suggestion is being generated. The quick model has one replaceable pending history snapshot; a result for an older snapshot is discarded when new transcript content arrives. There are two pending audio slots and one pending external transcript. Full audio queues discard chunks instead of blocking capture or growing memory. Chunks older than ten seconds are discarded. This favors responsiveness and does **not** produce a complete meeting transcript. There are no automatic retries. An HTTP stage has its configured total timeout, between 500 ms and 30 seconds. Stop closes audio immediately; an already-running HTTP request can take its remaining timeout to return, but cannot emit an answer. The app may terminate the helper after closing its input.

Recent conversation is bounded to about 16,000 characters, independent of the user context's 16,000-character limit. The built-in prompt asks for grounded, concise notes and a clarifying question when facts are missing. It cannot guarantee that a model will obey or that its answer is correct. Review suggestions before using them.

## Build and verify

```sh
cargo test --manifest-path conversation/Cargo.toml --locked
cargo build --release --manifest-path conversation/Cargo.toml --locked
SCREENCUE_LISTEN_BINARY=conversation/target/release/screencue-listen python -m unittest discover -s tests -p test_conversation.py -v
```

Linux development builds need ALSA and OpenSSL development libraries (`libasound2-dev` and `libssl-dev` on Debian/Ubuntu). The helper is a separate binary and does not add audio or HTTP dependencies to the lightweight overlay process. Tests use synthetic PCM, loopback HTTP fixtures and fake command adapters. They cover validation, framing, silence segmentation, bounded memory, provider formats, cancellation and refusal. Real devices, remote models and live meetings require an explicit user-run session.

Upstream implementation references: [CPAL supported platforms](https://github.com/RustAudio/cpal), [CPAL WASAPI loopback](https://github.com/RustAudio/cpal/blob/master/src/host/wasapi/mod.rs), [Microsoft loopback recording](https://learn.microsoft.com/en-us/windows/win32/coreaudio/loopback-recording), [OpenAI-compatible transcription shape](https://developers.openai.com/api/reference/resources/audio/subresources/transcriptions/methods/create), [OpenAI-compatible chat shape](https://developers.openai.com/api/reference/resources/chat/subresources/completions/methods/create).
