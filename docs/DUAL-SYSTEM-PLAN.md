# Fast decisions, deliberate guidance

Design and research record, updated on 2026-10-17. The [optional Python controller foundation](../controller/README.md) now implements candidate-based Choice adapters for Jev, OpenAI Decisions, and Ollama, an arbitrary-agent command adapter, direct display, a bounded two-stage router, and freshness/cancellation checks. These parts have offline tests; live providers and perceptual accuracy are not benchmarked. The native renderer is independently gaining text panels in the 0.0.3 development version; model code remains outside it. Automatic screen capture, element recognition, visual tracking, and client-plugin wiring remain proposed.

The goal is one short interaction: ask where something is, receive a cue promptly, and keep using the application. Model choice must not determine whether the renderer works. Existing agents can continue calling ScreenCue directly without this optional controller.

## What “System 1” means here

System 1 makes a bounded decision: select an observed target, check whether it is still present, or decide whether a new observation is needed. System 2 understands a difficult request, plans several steps, and writes explanations. Ordinary local code owns geometry, deadlines, cancellation, and drawing. Moving a known window must never require either model.

The model named by the user is **Jev from TypeSafe AI**. Its public HTTP interface is `POST https://api.typesafe.ai/v1/systemone`, authenticated with a bearer API key. Requests contain `model`, `state`, and named `questions`; answers preserve the question names. Choice selects an option, Noul estimates a yes/no probability, and Score evaluates an ordered rubric. See the [official API reference](https://docs.typesafe.ai/api).

Jev now also has **experimental image input and Box questions**. Box requires `jev-1.14.0` or `jev-preview` and the `TypeSafe-Danger-Zone-Images: true` header. It returns normalized edges, not desktop pixels. It always returns a box, including when the requested object is absent: ask a presence Noul in the same request and reject the box when presence is uncertain. TypeSafe explicitly describes these features as unsuitable for production guarantees. See [Box](https://docs.typesafe.ai/primitives/box), [Images](https://docs.typesafe.ai/images), and the [1.14 announcement](https://typesafe.ai/blog/jev-1-14).

For an initial stable route, provide observed controls as candidate IDs and let Jev select one, including a `none` option. Bind the result to the candidate's existing rectangle in code. This also works with text-only decision models. Text generation belongs to System 2 or a local label template.

The official model list documents hosted access and configurable model IDs. A public Jev weight download, redistribution license, or local installation procedure was not established by the reviewed official sources; do not advertise “local Jev.” Its SDK being open source does not establish that its model weights are. See [models](https://docs.typesafe.ai/models) and [official repositories](https://github.com/typesafe-ai).

## Adapter targets

These are verified upstream interfaces. The first three have **candidate/text-only controller implementations tested against fake endpoints**, and the command adapter is tested with local fixture processes. Native chat-generation and image adapters remain proposed. None is verified against a live paid model service in this work.

| Adapter | Intended use | Capabilities and boundary |
| --- | --- | --- |
| TypeSafe System One | Hosted Jev decisions | Native Choice/Noul/Score; separately enabled experimental image/Box route |
| OpenAI Decisions | Hosted System 1 | `POST /v1/decisions`, currently `gpt-6-luna`; predicate/choice/score over text or images; no Box type documented |
| Ollama System One | Local or explicitly selected cloud decision model | `POST /v1/systemone`; Nimble, Tev1, Clef, Clef Flash; model-specific image support and limits |
| OpenAI-compatible chat | Optional System 1 fallback or System 2 | Custom endpoint and model ID; JSON Schema when supported, bounded JSON validation otherwise |
| Native Anthropic / Gemini | System 2 or structured decision fallback | Provider-specific API adapter; do not assume their wire formats equal OpenAI's |
| Local command / supplied agent | Any model or existing agent harness | A versioned JSON decision contract over stdio; no vendor dependency |

OpenAI's current Decisions reference documents 1–200 questions and per-question refusals, which must remain distinct from a negative answer. See the [official Decisions reference](https://developers.openai.com/api/reference/resources/decisions/methods/create).

Ollama documents local decision inference with no API key. The decision route requires Ollama 0.35.0+, with Clef image support requiring 0.35.1+. Its choice limit is smaller than Jev's, so the adapter must enforce the declared limit or shortlist before inference. Images use an `images` array, unlike TypeSafe's asset references. A localhost endpoint can still route a `:cloud` model remotely; “local only” must check both endpoint and model. See [Ollama Decision](https://docs.ollama.com/capabilities/decision).

Local models remain separate downloads and processes. For example, Ollama lists Nimble as a 9B model with Apache-2.0 licensing; this is substantially larger than the overlay. Never bundle weights or start a GPU server as a prerequisite for showing a cue. Check the exact model revision and license before distributing it. See [Nimble](https://ollama.com/library/nimble).

TypeSafe itself publishes an optional LLM adapter supporting OpenAI-compatible endpoints, Anthropic, and Gemini. It is useful as a reference or evaluation dependency, but is unnecessary in the native renderer. A probability generated by an arbitrary LLM must not inherit Jev's calibration claims. See [System One Adapter](https://github.com/typesafe-ai/system-one-adapter-python). Local schema-constrained generation is also documented by [Ollama](https://docs.ollama.com/capabilities/structured-outputs) and [vLLM](https://docs.vllm.ai/en/latest/features/structured_outputs/).

## Keep the renderer small

The optional controller lives beside the existing binary. Its initial implementation accepts supplied observations, routes candidate decisions, and emits existing JSON `show` / `clear` commands to a caller-owned persistent transport. It does not yet own capture adapters or the native process. The longer-term design adds observation adapters and a target registry while retaining this separation. Provider SDKs, authentication, model weights, screenshot buffers, and inference runtimes stay outside `src/renderer.rs` and the native package.

```text
Voice / text / any agent
          |
  request + fresh observation
          |
  local target registry ---- native movement events
          |                         |
  bounded System 1 decision          |
          | uncertain               |
  optional System 2 plan             |
          |                         |
  validate + discard stale work <---+
          |
  persistent local MCP/JSON connection
          |
  ScreenCue draws and follows the target
```

The full controller contract should contain the following. The foundation implements monotonic deadlines, goal/observation generations, observed IDs/labels/rectangles, and decisions by candidate ID; display/window/element identities and capability negotiation remain follow-up work.

- Request identity: `request_id`, `intent_revision`, `observation_id`, and a monotonic deadline.
- Observation identity: capture time, display configuration revision, captured physical desktop rectangle, pixel dimensions, and native window identity when available.
- Candidate identity: stable element ID where provided, text/role, physical rectangle, source, and layout revision. A window title alone is not a stable ID.
- Result: `show`, `clear`, `need_observation`, or `escalate`; candidate ID or bounded image rectangle; optional short text; explicit absence/uncertainty; actual provider/model/version and timing.
- Adapter capabilities: accepted modalities, question types, maximum candidates, cancellation, local/remote data path, and confidence semantics. Unknown capability is not treated as supported.

These are proposed controller fields, not new native hint arguments. The controller converts a valid candidate decision into the renderer's hint schema. Its current command framing supports 128 KiB for text panels; raw screenshots still belong to a separate observation channel, not renderer commands.

## Reduce latency without losing the target

1. **Bypass inference for known actions.** Clear, native window movement, DPI conversion, display-edge placement, and known element geometry are local operations. An existing agent with a fresh target can draw directly.
2. **Reuse the running connection.** Do not spawn an interpreter, model, or new chat for each arrow. Keep network clients reusable while the optional controller is enabled.
3. **Observe narrowly.** Prefer an existing accessibility/DOM element identity; otherwise capture the relevant window or crop. Use the original pixel-to-desktop transform, including negative monitor origins. Do not guess scaling from a resized screenshot.
4. **Keep the newest observation.** Coalesce layout events, cancel superseded inference, and reject late answers with old intent/layout revisions. Controller `clear` already invalidates in-flight answers. The renderer's native hide shortcut still needs a cancellation event/epoch bridge before the same guarantee can apply to it.
5. **Track locally between decisions.** Window movement already follows locally. Add accessibility element events where available; use an optional local visual tracker for moving game/UI content. When identity is lost, show an honest edge/status cue or request a new observation instead of attaching to a nearby unrelated control.
6. **Make one bounded decision call.** Ask independent target/presence questions together. Do not request a paragraph before drawing. System 2 only runs on unresolved ambiguity or a task requiring planning.
7. **Avoid idle work.** No continuous capture while hidden. Adaptive observation rate while visible, dirty-region processing, and a bounded queue prevent a slow model from accumulating stale frames. A local model competing with a game for GPU time is optional and must be measured.

The current renderer polls tracking at roughly 16 ms on Windows and 33 ms elsewhere while active. Those intervals are not end-to-end latency guarantees. The largest perceived delay may be before the tool call: voice recognition, screen capture, host scheduling, network transfer, or reasoning. Measure those phases separately before optimizing drawing.

## Budgets and verification

Configure the inference deadline, maximum requests per second, image/token budget, escalation budget, and provider fallback explicitly. Retries must fit inside the remaining deadline and must never replay an obsolete observation. Keep the last valid cue when a provider times out; replace it only after the new target passes validation. If its target itself is stale, mark it for refresh.

Use separate latency measurements for speech-to-intent, observation, inference, queueing, validation, and first presented frame; report p50/p95 and cold versus warm starts. Also measure target error during movement/scrolling, false matches, hide latency, idle CPU, active CPU/GPU, memory, and game frame-time impact. Vendor timing claims are not ScreenCue measurements.

First acceptance cases: window drag; mixed-DPI monitor crossing; resize; scroll; two controls with the same label; missing target; target occlusion; provider timeout; late result after hide; rapid request replacement; and a game scene change. Include provider-specific refusal/invalid-output tests and live probes only when that provider is configured.

Observation is a separate, user-configured capability. A cloud adapter sends the selected text or pixels to that provider; local-only mode rejects remote adapters and cloud-routed model IDs. Credentials stay in environment variables or the OS credential store. Capture buffers are transient by default, and diagnostic timing logs should omit screenshots and private application text.

“Any model” means an extensible, validated adapter contract. It cannot make an unavailable API, a text-only model, an unsupported client surface, or an untested full-screen overlay automatically capable.
