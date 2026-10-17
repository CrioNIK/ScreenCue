# Optional decision controller

This Python 3.11+ foundation connects observed UI candidates to ScreenCue. It has no third-party dependencies and is separate from the native renderer. It does not capture the screen, discover controls, click, type, download models, or add itself to existing client plugins.

Implemented:

- Native typed Choice requests for TypeSafe Jev, OpenAI Decisions, and Ollama System One.
- A JSON command adapter for arbitrary local models, cloud clients, and agent wrappers.
- System 1 followed by at most one configured System 2 fallback. No automatic retries.
- Direct display of a known candidate without either model.
- One total deadline, observation freshness checks, generation checks, and serialized output.
- Clearing through the controller invalidates in-flight answers. New goals and observations invalidate old targets.
- A persistent JSON command stream compatible with `screencue --json`.

The provider formats and control logic have offline tests. **Live model accuracy, latency, credentials, and end-to-end screen perception have not been verified.** System 2 is a role: use a command wrapper around your chosen LLM or agent. This module does not yet implement native chat-generation APIs or Jev's experimental image/Box route.

## Import and use

Supply fresh, already-observed rectangles in physical desktop pixels. Negative monitor coordinates are valid. The model receives candidate IDs, labels, and roles; coordinates come only from the observation.

```python
import time
from controller.screencue_controller import Candidate, Controller, Observation

# Replace this with a quick write/enqueue to your persistent ScreenCue transport.
commands = []
controller = Controller(emit=commands.append)
observation = Observation(
    id="frame-17",
    candidates=(Candidate("settings", "Open settings", 900, 140, 96, 32, "button"),),
    captured_at=time.monotonic(),
)
ticket = controller.begin("Show settings", observation)
controller.show(ticket, "settings")  # Direct path: zero model calls.
controller.clear()                   # Invalidates pending decisions before clearing.
```

`captured_at` uses the same process's monotonic clock. Account for capture age when accepting observations from another process; do not substitute wall-clock time. `controller.observe(new_observation)` keeps the goal while invalidating an earlier layout. A new goal or observation clears an already-emitted prior cue so an unresolved new layout cannot leave an obsolete target on screen. Native window movement needs no replacement observation; the renderer already follows it locally. Call `controller.resolve(ticket)` to use configured adapters instead of `show`.

Pass `presentation={...}` to `show` or `resolve` to customize the text panel independently of the target. Supported fields are `title` (100 characters), `text` (16,000 characters), optional `panel_x` and `panel_y` together (physical desktop coordinates), `panel_width` (120–4,000), `text_page` (zero-based, up to `4294967295`), and boolean `arrow` / `target`. Setting both booleans to false gives a text-only panel. The native renderer clamps placement and page number; its status/reply reports `panel` geometry, `page`, and `page_count`. Presentation content comes from the caller, is validated before inference, and is not sent to the decision model.

Each ticket can resolve only once. A valid candidate causes a `show` command with its original rectangle; unknown candidates, absence, refusals, and low-confidence answers cannot invent a target. `Outcome.status == "emitted"` means the command reached the configured sink, **not that a native frame was visibly presented**. Inspect renderer replies through the transport you own.

The sink must be quick and preserve command order. It runs under the same commit lock as `clear`; do not wait for a model or renderer acknowledgment inside it. The caller owns the long-lived renderer process and its reply handling. Closing the renderer's stdin removes its overlay.

## Explicit provider configuration

No provider is enabled by default. Configuration contains an endpoint, model ID, and an environment-variable name for an existing key; never put a key value in this file.

```json
{
  "system1": {
    "kind": "typesafe",
    "endpoint": "https://api.typesafe.ai/v1/systemone",
    "model": "jev-1.13.0",
    "key_env": "TYPESAFE_API_KEY"
  },
  "policy": {
    "total_timeout_ms": 1000,
    "system1_timeout_ms": 500,
    "max_observation_age_ms": 1500,
    "system1_min_confidence": 0.7,
    "system2_min_confidence": 0.7
  }
}
```

Alternative adapter entries:

| `kind` | `endpoint` | Example `model` | Credential |
| --- | --- | --- | --- |
| `typesafe` | `https://api.typesafe.ai/v1/systemone` | `jev-1.13.0` | Explicit `key_env` |
| `openai-decisions` | `https://api.openai.com/v1/decisions` | `gpt-6-luna` | Explicit `key_env` |
| `ollama` | `http://localhost:11434/v1/systemone` | `nimble` | None required locally |

Use `"local_only": true` for a loopback Ollama endpoint and a local model. This rejects remote endpoints and `:cloud` model IDs. It cannot inspect whether a custom local service or command wrapper forwards data elsewhere. The endpoint/runtime remains an explicit trust choice. A model must already be available in the configured Ollama runtime; no installation or weight download is performed.

This initial implementation accepts at most 254 observed candidates for Jev/OpenAI and 25 for Ollama, reserving one choice for no match. Shortlist larger observations before calling. Model request and response bodies are limited to 64 KiB; renderer command lines allow up to 128 KiB, including longer panel text. HTTP redirects are rejected, including redirects to another host. Only HTTPS and loopback HTTP endpoints are accepted.

The standard-library HTTP adapter does not provide a persistent TLS connection pool. Network setup remains part of measured latency; reusable transport clients are follow-up work.

Thresholds above are starting settings, not accuracy guarantees. Confidence values have different meanings across providers and must be evaluated on your UI tasks. The total deadline includes both stages; System 2 receives only the remaining budget. Provider retries are disabled. Generation changes are checked while waiting so a superseded resolver can finish without waiting for its provider. A timed-out worker cannot emit a cue; at most one call per adapter object can remain outstanding until its transport returns. New work can use the other stage or remain unresolved while that adapter is busy. Python cannot forcibly interrupt an arbitrary blocking callback, so custom adapters must honor their timeout.

## Any model or agent through a command

Set either stage to a wrapper executable and arguments. The wrapper may use an already-running local model or a cloud SDK. ScreenCue does not choose its credentials, model, or remote destination.

```json
{
  "system2": {
    "kind": "command",
    "command": ["/absolute/path/python", "/absolute/path/my_agent_adapter.py"]
  }
}
```

The child receives one UTF-8 JSON object on stdin, followed by EOF:

```json
{"version":1,"goal":"Find settings","observation_id":"frame-17","candidates":[{"id":"settings","label":"Settings","role":"button"}],"timeout_ms":500}
```

It returns exactly one object on stdout:

```json
{"candidate_id":"settings","confidence":0.94,"refused":false}
```

Use `"candidate_id": null` when unresolved. Coordinates, renderer commands, and additional output fields are rejected. Diagnostics must not go to stdout. No shell is used; arguments are passed as an array. Each decision starts one wrapper process, so process startup adds latency. Keep heavyweight inference in a separately managed server. The wrapper must clean up its own descendants; the adapter can terminate only its direct child. The bounded request uses a temporary file that is removed when the invocation ends; command stderr is discarded.

## JSON stream runner

```sh
python controller/screencue_controller.py --config /absolute/path/controller.json
```

Keep stdin open. Send one object per line; stdout contains only renderer-compatible `show`/`clear` commands and stderr contains short status records. Connect stdout to a persistent `screencue --json` process using your host's pipes. The runner does not own or launch the renderer.

Direct display requires no provider configuration:

```json
{"command":"show","goal":"Find settings","observation_id":"frame-17","candidate_id":"settings","observed_age_ms":0,"candidates":[{"id":"settings","label":"Open settings","x":900,"y":140,"width":96,"height":32,"role":"button"}]}
```

Use `"command":"decide"` and omit `candidate_id` to select through the configured stages. The runner processes one decision at a time and retains only the latest pending observation. `{"command":"clear"}` invalidates pending answers immediately. EOF clears the cue; piping a completed request file into the runner is therefore not a persistent demonstration.

The stream accepts the same presentation fields at the command's top level, for example `"title":"Next step","text":"A longer explanation…","panel_width":640,"arrow":false,"target":false`.

The renderer's native hide shortcut currently has no event channel back to this controller. **Call `controller.clear()` or send a controller `clear` when cancelling pending inference.** Pressing the native shortcut hides the current cue, but this foundation cannot yet guarantee that a later new controller command will not display another cue. Integrating a renderer cancellation epoch is follow-up work.

## Offline verification

```sh
python tests/test_controller.py -v
```

Tests cover native request shapes, absence/refusal/fallback, old observation and goal rejection, explicit clear during inference, deadline and concurrency limits, geometry validation, redirected credentials, bounded command output, child timeout, and clean JSON output. Fake HTTP servers bind only to loopback; tests do not read real API credentials or call model services.

See [the design and source record](../docs/DUAL-SYSTEM-PLAN.md) for upstream references and remaining perception/tracking work.
