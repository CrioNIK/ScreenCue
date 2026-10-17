"""Offline tests: no microphone, external provider or real API credentials."""
import http.server
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from controller.conversation_orchestrator import (
    ChatHTTP, CommandJSON, ControllerError, Decision, Orchestrator, Policy,
    decision_provider, load_config, partial_answer, main, NONE,
)


def until(predicate, seconds=2):
    end = time.monotonic() + seconds
    while time.monotonic() < end:
        if predicate():
            return
        time.sleep(0.005)
    raise AssertionError("Condition did not become true before its deadline")


class Provider:
    def __init__(self, callback):
        self.callback = callback
        self.calls = []
        self.started = threading.Event()

    def generate(self, payload, timeout, cancelled):
        self.calls.append(payload)
        self.started.set()
        return self.callback(payload, timeout, cancelled)


class Chooser:
    def __init__(self, choice="action:clarify", confidence=.95):
        self.choice, self.confidence = choice, confidence
        self.calls = []

    def decide(self, ticket, timeout):
        self.calls.append(ticket)
        return Decision(self.choice, self.confidence)


def answer(value="Ask which milestone matters most."):
    return {"text": value, "confidence": .9, "fact_ids": [], "refused": False}


class OrchestratorTests(unittest.TestCase):
    def setUp(self):
        self.sessions = []
        self.gates = []

    def tearDown(self):
        for session in self.sessions:
            session.clear(stop=True)
        for gate in self.gates:
            gate.set()
        for session in self.sessions:
            for lane in session.lanes.values():
                lane.thread.join(timeout=2)

    def session(self, **kwargs):
        events = []
        session = Orchestrator(events.append, **kwargs)
        self.sessions.append(session)
        return session, events

    def gate(self):
        gate = threading.Event()
        self.gates.append(gate)
        return gate

    def test_fast_does_not_wait_for_slow_planner(self):
        gate = self.gate()
        planner = Provider(lambda p, t, c: (gate.wait(t), {"candidates": []})[1])
        fast = Provider(lambda p, t, c: answer())
        session, events = self.session(fast=fast, planner=planner)
        session.transcript({"text": "What comes next?", "speaker_role": "investor"})
        until(lambda: any(e["event"] == "hint" for e in events))
        self.assertFalse(gate.is_set())
        self.assertEqual(next(e for e in events if e["event"] == "hint")["lane"], "fast")

    def test_decider_is_independent_of_both_language_models(self):
        gate = self.gate()
        fast = Provider(lambda p, t, c: (gate.wait(t), answer())[1])
        planner = Provider(lambda p, t, c: (gate.wait(t), {"candidates": []})[1])
        chooser = Chooser()
        session, events = self.session(fast=fast, planner=planner, decider=chooser)
        session.transcript({"text": "Could you explain?"})
        until(lambda: any(e["event"] == "hint" for e in events))
        cue = next(e for e in events if e["event"] == "hint")
        self.assertEqual(cue["lane"], "decider")
        self.assertFalse(gate.is_set())
        ids = [c.id for c in chooser.calls[0].observation.candidates]
        self.assertEqual(ids, ["action:wait", "action:clear", "action:clarify"])

    def test_role_mapping_does_not_infer_identity_from_audio_source(self):
        fast = Provider(lambda p, t, c: answer())
        session, events = self.session(fast=fast, assisted_role="presenter",
            speaker_roles={"microphone:A": "presenter", "system_audio:B": "investor"})
        session.transcript({"text": "I will explain", "source": "microphone", "speaker_id": "A"})
        self.assertFalse(fast.calls)
        session.transcript({"text": "What does it cost?", "source": "system_audio", "speaker_id": "B"})
        until(lambda: len(fast.calls) == 1)
        roles = [t["speaker_role"] for t in fast.calls[0]["turns"]]
        self.assertEqual(roles, ["presenter", "investor"])
        session.transcript({"text": "An unattributed voice", "source": "microphone"})
        latest = next(reversed(session.history.values()))
        self.assertIsNone(latest.speaker_role)

    def test_revisions_replace_partial_and_final_is_not_rewritten(self):
        session, _ = self.session()
        self.assertTrue(session.transcript({"id": "t", "revision": 1, "final": False, "text": "What is"}))
        self.assertFalse(session.transcript({"id": "t", "revision": 1, "text": "Duplicate"}))
        self.assertTrue(session.transcript({"id": "t", "revision": 2, "final": True, "text": "What is your traction?"}))
        self.assertFalse(session.transcript({"id": "t", "revision": 3, "text": "Bad replacement"}))
        self.assertEqual(len(session.history), 1)
        self.assertEqual(session.history["t"].text, "What is your traction?")

    def test_latest_pending_is_bounded_and_old_fast_answer_is_dropped(self):
        gate = self.gate()
        fast = Provider(lambda p, t, c: (gate.wait(t), answer(p["turns"][-1]["text"]))[1])
        session, events = self.session(fast=fast, policy=Policy(fast_timeout_ms=3000))
        session.transcript({"id": "first", "text": "First question"})
        self.assertTrue(fast.started.wait(1))
        for index in range(25):
            session.transcript({"id": f"q-{index}", "text": f"Question {index}"})
        lane = session.lanes["fast"]
        self.assertEqual(len(fast.calls), 1)
        self.assertEqual(lane.pending.payload["turns"][-1]["text"], "Question 24")
        self.assertLessEqual(len(session.history), session.policy.max_turns)
        gate.set()
        until(lambda: any(e["event"] == "hint" for e in events))
        hints = [e for e in events if e["event"] == "hint"]
        self.assertEqual([e["text"] for e in hints], ["Question 24"])
        self.assertEqual(len(fast.calls), 2)

    def test_context_change_and_stop_suppress_inflight_answers(self):
        for operation in ("context", "clear", "stop"):
            with self.subTest(operation=operation):
                gate = self.gate()
                fast = Provider(lambda p, t, c: (gate.wait(t), answer("Old target"))[1])
                session, events = self.session(fast=fast)
                session.transcript({"text": "Old question"})
                self.assertTrue(fast.started.wait(1))
                if operation == "context":
                    session.context({"objective": "New target", "facts": {"new": "Supplied fact"}})
                else:
                    session.clear(stop=operation == "stop")
                gate.set()
                until(lambda: session.lanes["fast"].active_cancel is None)
                self.assertFalse(any(e["event"] == "hint" for e in events))

    def test_planner_bank_never_directly_displays_and_unknown_facts_are_rejected(self):
        planner = Provider(lambda p, t, c: {"candidates": [{"id": "traction", "kind": "response",
            "text": "We have three pilots.", "when": "Asked about traction", "fact_ids": ["f1"]}]})
        session, events = self.session(planner=planner, facts={"f1": "We have three pilot customers."})
        session.transcript({"text": "Tell me about the company"})
        until(lambda: len(session.bank) == 1)
        self.assertFalse(any(e["event"] == "hint" for e in events))
        with self.assertRaises(ControllerError):
            session._fast_answer({"text": "Unsupported fact", "confidence": .99, "fact_ids": ["invented"]})
        self.assertEqual(session._fast_answer({"text": "Weak", "confidence": .1}), "")

    def test_new_transcript_clears_existing_cue_and_ttl_expires(self):
        session, events = self.session(policy=Policy(hint_ttl_ms=500))
        session.direct_hint({"text": "Old cue"})
        session.transcript({"text": "A new question"})
        self.assertEqual(events[-2]["event"], "clear")
        session.direct_hint({"text": "Short-lived cue"})
        session.visible_until = time.monotonic() - .01
        session.poll()
        self.assertEqual(events[-1]["event"], "clear")

    def test_screen_hint_requires_fresh_observed_geometry(self):
        session, events = self.session()
        event = {"observation_id": "frame-1", "observed_age_ms": 10,
            "candidate": {"id": "button", "label": "Open", "x": -150, "y": 20, "width": 80, "height": 30}}
        session.screen_hint(event)
        self.assertEqual(events[-1]["event"], "screen_hint")
        self.assertEqual(events[-1]["hint"]["x"], -150)
        self.assertLess(abs(events[-1]["captured_at_ms"] - time.time()*1000), 100)
        with self.assertRaises(ControllerError):
            session.screen_hint({**event, "observed_age_ms": 5000})
        with self.assertRaises(ControllerError):
            session.screen_hint({**event, "candidate": {"x": 0}})

    def test_decider_reuses_typed_api_with_conversation_instructions(self):
        decider = decision_provider({"kind": "ollama", "endpoint": "http://localhost:11434/v1/systemone", "model": "installed-model"})
        chooser = Chooser()
        session, _ = self.session(decider=chooser)
        session.transcript({"text": "What is next?"})
        until(lambda: bool(chooser.calls))
        request = decider.request(chooser.calls[0])
        self.assertIn("prepared speaking cue", request["questions"]["target"]["instructions"])
        self.assertIn("action:clarify", request["questions"]["target"]["criteria"])
        self.assertNotIn("x", request["state"]["candidates"][0])

    def test_decider_sees_full_prepared_text_and_fact_ids(self):
        chooser = Chooser("prepared")
        session, _ = self.session(decider=chooser, facts={"fact": "A supplied fact"})
        session.bank = [{"id": "prepared", "kind": "response", "text": "z"*480,
                         "when": "a"*250, "fact_ids": ["fact"]}]
        session.bank_at = time.monotonic()
        session.transcript({"text": "What is next?"})
        until(lambda: bool(chooser.calls))
        candidate = json.loads(chooser.calls[0].goal)["prepared_candidates"][-1]
        self.assertEqual(candidate["text"], "z"*480)
        self.assertEqual(candidate["fact_ids"], ["fact"])
        self.assertEqual(candidate["when"], "a"*250)
        with self.assertRaises(ControllerError):
            session._plan({"candidates": [{"id": NONE, "kind": "question", "text": "Question?", "fact_ids": []}]})

    def test_late_decider_does_not_replace_an_already_valid_fast_answer(self):
        gate = self.gate()
        class DelayedChoice:
            def decide(self, ticket, timeout):
                gate.wait(timeout)
                return Decision("action:clarify", .99)
        fast = Provider(lambda p, t, c: answer("A current fast answer"))
        session, events = self.session(fast=fast, decider=DelayedChoice())
        session.transcript({"text": "What is next?"})
        until(lambda: any(e["event"] == "hint" for e in events))
        gate.set()
        until(lambda: any(e.get("lane") == "decider" and e["event"] == "metrics" for e in events))
        self.assertEqual([e["text"] for e in events if e["event"] == "hint"], ["A current fast answer"])

    def test_decider_clear_cancels_the_current_fast_answer(self):
        gate = self.gate()
        fast = Provider(lambda p, t, c: (gate.wait(t), answer("Must never return"))[1])
        session, events = self.session(fast=fast, decider=Chooser("action:clear"))
        session.transcript({"text": "No cue is needed"})
        until(lambda: any(e["event"] == "clear" for e in events))
        gate.set()
        until(lambda: session.lanes["fast"].active_cancel is None)
        self.assertFalse(any(e["event"] == "hint" for e in events))

    def test_invalid_final_stream_result_clears_only_its_own_current_prefix(self):
        for result, error in (({"text": "", "confidence": .9, "refused": True}, None),
                              ({"text": "bad", "confidence": .99, "fact_ids": ["unknown"]}, None),
                              (None, "TransportError")):
            with self.subTest(result=result, error=error):
                session, events = self.session()
                session.transcript({"text": "What is next?"})
                work = session._work("fast", session._payload("suggest"))
                session._partial(work, answer("A current partial thought is visible"), threading.Event())
                session._complete("fast", work, result, error, False, 1)
                self.assertIsNone(session.published)
                self.assertTrue(any(e["event"] == "clear" for e in events))
                session._hint("Another valid cue", "Next", "agent")
                count = len([e for e in events if e["event"] == "clear"])
                session._complete("fast", work, result, error, False, 1)
                self.assertEqual(len([e for e in events if e["event"] == "clear"]), count)
                self.assertEqual(session.published, (session.generation, "agent"))

    def test_streaming_requires_valid_ordered_prefix(self):
        self.assertIsNone(partial_answer('{"text":"premature text'))
        partial = partial_answer('{"confidence":0.91,"fact_ids":[],"refused":false,"text":"A helpful partial thought')
        self.assertEqual(partial["text"], "A helpful partial thought")
        partial = partial_answer('{"confidence":0.9,"fact_ids":[],"refused":false,"text":"A thought with \\u00')
        self.assertEqual(partial["text"], "A thought with ")

    def test_sse_stream_emits_before_completion_and_stop_blocks_later_chunks(self):
        first_sent, release = threading.Event(), self.gate()
        chunks = ['{"confidence":0.9,"fact_ids":[],"refused":false,"text":"A useful thought is ready to read', ' before the response ends."}']
        class Handler(http.server.BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass
            def do_POST(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                assert body["stream"] is True
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.end_headers()
                try:
                    for index, chunk in enumerate(chunks):
                        if index:
                            release.wait(2)
                        packet = json.dumps({"choices": [{"delta": {"content": chunk}}]})
                        self.wfile.write(("data: " + packet + "\n\n").encode())
                        self.wfile.flush()
                        if not index:
                            first_sent.set()
                    self.wfile.write(b"data: [DONE]\n\n")
                    self.wfile.flush()
                except (BrokenPipeError, ConnectionResetError):
                    pass
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            provider = ChatHTTP(endpoint=f"http://127.0.0.1:{server.server_port}/chat", model="test", streaming=True)
            session, events = self.session(fast=provider)
            session.transcript({"text": "What is next?"})
            self.assertTrue(first_sent.wait(1))
            until(lambda: any(e.get("lane") == "fast_stream" and e["event"] == "hint" for e in events))
            before = len([e for e in events if e["event"] == "hint"])
            session.clear(stop=True)
            release.set()
            until(lambda: session.lanes["fast"].active_cancel is None)
            self.assertEqual(len([e for e in events if e["event"] == "hint"]), before)
        finally:
            release.set()
            server.shutdown()
            server.server_close()

    def test_command_provider_cancels_its_direct_child(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "adapter.py"
            path.write_text("import time\ntime.sleep(10)\n", encoding="utf-8")
            provider = CommandJSON([sys.executable, str(path)])
            cancelled = threading.Event()
            cancelled.set()
            started = time.monotonic()
            with self.assertRaises(ControllerError):
                provider.generate({"operation": "suggest"}, 10, cancelled)
            self.assertLess(time.monotonic() - started, .5)

    def test_stop_synchronously_reaps_active_command_child(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / "adapter.py"
            path.write_text("import time\ntime.sleep(30)\n", encoding="utf-8")
            provider = CommandJSON([sys.executable, str(path)])
            session, _ = self.session(fast=provider, policy=Policy(fast_timeout_ms=30000))
            session.transcript({"text": "What is next?"})
            until(lambda: bool(provider.active_children()))
            children = provider.active_children()
            session.clear(stop=True)
            self.assertTrue(all(child.poll() is not None for child in children))

    def test_validation_checks_configured_native_helper_too(self):
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            (folder / "native.json").write_text('{"suggestion":null}', encoding="utf-8")
            config = folder / "advanced.json"
            config.write_text(json.dumps({"engine": "conversation-orchestrator", "helper_config": "native.json"}), encoding="utf-8")
            with mock.patch("controller.conversation_orchestrator.subprocess.run", return_value=subprocess.CompletedProcess([], 1)) as checked:
                with mock.patch("sys.stdout"):
                    result = main(["--config", str(config), "--helper", str(folder / "helper"), "--validate-config"])
                self.assertEqual(result, 1)
                self.assertEqual(checked.call_args.args[0][1], "--validate-config")

    def test_cli_direct_events_and_configuration_validation_are_provider_free(self):
        script = Path(__file__).resolve().parents[1] / "controller" / "conversation_orchestrator.py"
        with tempfile.TemporaryDirectory() as directory:
            config = Path(directory) / "config.json"
            config.write_text(json.dumps({"engine": "conversation-orchestrator", "facts": {}, "fast": None, "planner": None, "decider": None}), encoding="utf-8")
            kwargs, helper = load_config(config)
            self.assertIsNone(helper)
            self.assertIsNone(kwargs["fast"])
            result = subprocess.run([sys.executable, str(script), "--config", str(config), "--start"],
                input='{"event":"hint","text":"Direct agent cue"}\n{"command":"stop"}\n',
                text=True, capture_output=True, timeout=8)
            self.assertEqual(result.returncode, 0, result.stderr)
            events = [json.loads(line) for line in result.stdout.splitlines()]
            self.assertTrue(any(e.get("text") == "Direct agent cue" for e in events))
            self.assertEqual(events[-1], {"event": "status", "state": "stopped"})


if __name__ == "__main__":
    unittest.main()
