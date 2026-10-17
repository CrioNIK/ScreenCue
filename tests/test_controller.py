"""Offline tests: fake HTTP/commands, no API credentials, AI clients, or capture."""
from __future__ import annotations

from contextlib import contextmanager
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest import mock

ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("screencue_controller", ROOT / "controller/screencue_controller.py")
sc = importlib.util.module_from_spec(SPEC)
sys.modules[SPEC.name] = sc
SPEC.loader.exec_module(sc)


class Fixed:
    def __init__(self, decision):
        self.decision, self.calls = decision, []

    def decide(self, ticket, timeout):
        self.calls.append((ticket, timeout))
        return self.decision


class Blocked(Fixed):
    def __init__(self, decision):
        super().__init__(decision)
        self.entered, self.release = threading.Event(), threading.Event()

    def decide(self, ticket, timeout):
        self.calls.append((ticket, timeout))
        self.entered.set()
        self.release.wait(2)
        return self.decision


@contextmanager
def endpoint(response, status=200, headers=None):
    requests = []

    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            raw = self.rfile.read(int(self.headers["Content-Length"]))
            requests.append({"path": self.path, "body": json.loads(raw), "headers": dict(self.headers)})
            self.send_response(status)
            for key, value in (headers or {}).items():
                self.send_header(key, value)
            self.end_headers()
            self.wfile.write(response if isinstance(response, bytes) else json.dumps(response).encode())

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.01}, daemon=True)
    thread.start()
    try:
        yield f"http://127.0.0.1:{server.server_port}/v1/fake", requests
    finally:
        server.shutdown()
        server.server_close()
        thread.join(timeout=1)


class Fixtures:
    def setUp(self):
        self.output = []
        self.candidate = sc.Candidate("settings", "Open settings", -950, 130, 96, 30, "button")

    def observation(self, *, captured_at=None, candidate=None, id="frame-1"):
        return sc.Observation(id, (candidate or self.candidate,),
                              time.monotonic() if captured_at is None else captured_at)

    def controller(self, first=None, second=None, **kwargs):
        return sc.Controller(self.output.append, first, second, **kwargs)


class ControllerTests(Fixtures, unittest.TestCase):
    def test_direct_show_preserves_observed_physical_rectangle_without_model(self):
        adapter = Fixed(sc.Decision("invented", 1))
        controller = self.controller(adapter)
        ticket = controller.begin("Find settings", self.observation())
        outcome = controller.show(ticket, "settings", "")
        self.assertEqual(outcome.status, "emitted")
        self.assertEqual(adapter.calls, [])
        self.assertEqual(self.output, [{"command": "show", "x": -950, "y": 130,
            "width": 96, "height": 30, "style": "box", "shape": "rounded",
            "arrow": True, "follow_window": True, "text": "", "duration_ms": 0}])

    def test_strong_system1_result_skips_system2(self):
        first, second = Fixed(sc.Decision("settings", .95)), Fixed(sc.Decision("settings", .99))
        controller = self.controller(first, second)
        outcome = controller.resolve(controller.begin("Find settings", self.observation()))
        self.assertEqual((outcome.status, outcome.stage), ("emitted", "system1"))
        self.assertEqual(len(first.calls), 1)
        self.assertEqual(second.calls, [])

    def test_low_confidence_unknown_none_and_refusal_get_only_one_fallback(self):
        for decision in (sc.Decision("settings", .1), sc.Decision("invented", 1),
                         sc.Decision(None, 1), sc.Decision("settings", 1, True)):
            with self.subTest(decision=decision):
                first, second = Fixed(decision), Fixed(sc.Decision("settings", .95))
                controller = self.controller(first, second)
                ticket = controller.begin("Find settings", self.observation())
                self.assertEqual(controller.resolve(ticket).stage, "system2")
                self.assertEqual((len(first.calls), len(second.calls)), (1, 1))
                controller.resolve(ticket)
                self.assertEqual((len(first.calls), len(second.calls)), (1, 1))

    def test_no_match_from_both_stages_never_draws_or_retries(self):
        first, second = Fixed(sc.Decision(None, 1)), Fixed(sc.Decision("unknown", 1))
        controller = self.controller(first, second)
        ticket = controller.begin("Find settings", self.observation())
        self.assertEqual(controller.resolve(ticket).status, "unresolved")
        self.assertEqual(controller.resolve(ticket).reason, "already_attempted")
        self.assertEqual(self.output, [])
        self.assertEqual((len(first.calls), len(second.calls)), (1, 1))

    def test_clear_during_inference_prevents_late_resurrection(self):
        adapter = Blocked(sc.Decision("settings", .99))
        self.addCleanup(adapter.release.set)
        controller = self.controller(adapter)
        ticket = controller.begin("Find settings", self.observation())
        results = []
        thread = threading.Thread(target=lambda: results.append(controller.resolve(ticket)))
        thread.start()
        self.assertTrue(adapter.entered.wait(1))
        controller.clear()
        adapter.release.set()
        thread.join(1)
        self.assertFalse(thread.is_alive())
        self.assertEqual(self.output, [{"command": "clear"}])
        self.assertEqual(results[0].reason, "superseded")

    def test_new_observation_rejects_old_target_even_when_id_is_reused(self):
        adapter = Blocked(sc.Decision("settings", .99))
        self.addCleanup(adapter.release.set)
        controller = self.controller(adapter)
        old = controller.begin("Find settings", self.observation())
        thread = threading.Thread(target=lambda: controller.resolve(old))
        thread.start()
        self.assertTrue(adapter.entered.wait(1))
        moved = sc.Candidate("settings", "Open settings", 700, 400, 100, 40)
        current = controller.observe(self.observation(candidate=moved, id="frame-2"))
        self.assertEqual(controller.show(current, "settings").status, "emitted")
        adapter.release.set()
        thread.join(1)
        self.assertEqual(len(self.output), 1)
        self.assertEqual((self.output[0]["x"], self.output[0]["y"]), (700, 400))

    def test_new_goal_invalidates_old_ticket(self):
        controller = self.controller()
        old = controller.begin("Settings", self.observation())
        controller.begin("A different goal", self.observation())
        self.assertEqual(controller.show(old, "settings").reason, "superseded")
        self.assertEqual(self.output, [])

    def test_unresolved_new_layout_clears_an_already_emitted_old_rectangle(self):
        controller = self.controller(Fixed(sc.Decision(None, 1)))
        ticket = controller.begin("Settings", self.observation())
        controller.show(ticket, "settings")
        moved = sc.Candidate("settings", "Settings", 700, 400, 100, 40)
        new_ticket = controller.observe(self.observation(candidate=moved, id="frame-2"))
        self.assertEqual(controller.resolve(new_ticket).status, "unresolved")
        self.assertEqual([c["command"] for c in self.output], ["show", "clear"])

    def test_superseded_resolver_exits_before_its_slow_adapter_finishes(self):
        adapter = Blocked(sc.Decision("settings", .99))
        self.addCleanup(adapter.release.set)
        controller = self.controller(adapter, policy=sc.Policy(total_timeout_ms=2000, system1_timeout_ms=1500))
        old = controller.begin("Find settings", self.observation())
        results = []
        thread = threading.Thread(target=lambda: results.append(controller.resolve(old)))
        thread.start()
        self.assertTrue(adapter.entered.wait(1))
        current = controller.begin("New goal", self.observation(id="frame-2"))
        thread.join(.5)
        self.assertFalse(thread.is_alive())
        self.assertFalse(adapter.release.is_set())
        self.assertEqual(results[0].reason, "superseded")
        self.assertEqual(controller.show(current, "settings").status, "emitted")
        adapter.release.set()

    def test_deadline_and_observation_age_checked_before_and_after_adapter(self):
        now = [100.]
        adapter = Fixed(sc.Decision("settings", 1))
        controller = self.controller(adapter, clock=lambda: now[0])
        ticket = controller.begin("Settings", self.observation(captured_at=100.))
        now[0] = 101.
        self.assertEqual(controller.resolve(ticket).reason, "deadline")
        self.assertEqual(adapter.calls, [])
        ticket = controller.begin("Settings", self.observation(captured_at=99.))
        self.assertEqual(controller.resolve(ticket).reason, "stale_observation")
        ticket = controller.begin("Settings", self.observation(captured_at=102.))
        self.assertEqual(controller.resolve(ticket).reason, "stale_observation")

        class Advances:
            def decide(self, ticket, timeout):
                now[0] = ticket.deadline + .1
                return sc.Decision("settings", 1)

        controller = self.controller(Advances(), clock=lambda: now[0])
        ticket = controller.begin("Settings", self.observation(captured_at=now[0]))
        self.assertEqual(controller.resolve(ticket).reason, "deadline")
        self.assertEqual(self.output, [])

    def test_timeout_budget_and_outstanding_adapter_cap(self):
        adapter = Blocked(sc.Decision("settings", .99))
        self.addCleanup(adapter.release.set)
        policy = sc.Policy(total_timeout_ms=120, system1_timeout_ms=40)
        controller = self.controller(adapter, policy=policy)
        started = time.monotonic()
        controller.resolve(controller.begin("Settings", self.observation()))
        elapsed = time.monotonic() - started
        self.assertLess(elapsed, .35)
        self.assertTrue(adapter.entered.is_set())
        controller.resolve(controller.begin("Settings again", self.observation()))
        self.assertEqual(len(adapter.calls), 1)
        self.assertEqual(self.output, [])
        adapter.release.set()

    def test_fallback_uses_remaining_budget_and_never_adds_full_second_budget(self):
        now = [100.]

        class SlowFirst:
            def decide(self, ticket, timeout):
                now[0] += .4
                return sc.Decision(None, 1)

        second = Fixed(sc.Decision("settings", .9))
        controller = self.controller(SlowFirst(), second, clock=lambda: now[0])
        controller.resolve(controller.begin("Settings", self.observation(captured_at=100.)))
        self.assertAlmostEqual(second.calls[0][1], .6)

    def test_duplicate_concurrent_resolve_does_not_spend_second_budget(self):
        adapter = Blocked(sc.Decision("settings", .99))
        fallback = Fixed(sc.Decision("settings", .99))
        self.addCleanup(adapter.release.set)
        controller = self.controller(adapter, fallback)
        ticket = controller.begin("Find settings", self.observation())
        thread = threading.Thread(target=lambda: controller.resolve(ticket))
        thread.start()
        self.assertTrue(adapter.entered.wait(1))
        self.assertEqual(controller.resolve(ticket).reason, "already_attempted")
        self.assertEqual(fallback.calls, [])
        adapter.release.set()
        thread.join(1)

    def test_invalid_geometry_decisions_and_policy_are_rejected(self):
        for kwargs in ({"x": True}, {"width": 0}, {"height": 4001},
                       {"x": 2**31 - 1}, {"id": sc.NONE}):
            values = dict(id="a", label="A", x=0, y=0, width=20, height=20)
            values.update(kwargs)
            with self.subTest(kwargs=kwargs), self.assertRaises(sc.ControllerError):
                sc.Candidate(**values)
        for value in (float("nan"), float("inf"), True, -1, 1.1):
            with self.subTest(value=value), self.assertRaises(sc.ControllerError):
                sc.Decision("a", value)
        with self.assertRaises(sc.ControllerError):
            sc.Observation("frame", (self.candidate, self.candidate), time.monotonic())
        with self.assertRaises(sc.ControllerError):
            sc.Policy(total_timeout_ms=100, system1_timeout_ms=200)

    def test_model_state_contains_no_coordinate_generation_task(self):
        controller = self.controller()
        ticket = controller.begin("Settings", self.observation())
        state = ticket.state()
        self.assertEqual(set(state["candidates"][0]), {"id", "label", "role"})
        self.assertNotIn("x", state["candidates"][0])

    def test_rich_panel_passthrough_keeps_target_and_model_input_separate(self):
        adapter = Fixed(sc.Decision("settings", .99))
        controller = self.controller(adapter)
        ticket = controller.begin("Find settings", self.observation())
        presentation = {"title": "A useful explanation", "text": "Read this paragraph.\n" * 500,
            "panel_x": -700, "panel_y": 240, "panel_width": 640,
            "text_page": 3, "arrow": False, "target": False}
        self.assertEqual(controller.resolve(ticket, presentation).status, "emitted")
        for name, value in presentation.items():
            self.assertEqual(self.output[0][name], value)
        self.assertEqual(self.output[0]["x"], -950)
        self.assertNotIn("text", adapter.calls[0][0].state())

    def test_invalid_panel_is_rejected_before_spending_a_request(self):
        for value in ({"title": "x" * 101}, {"text": "x" * 16001},
                      {"text": "bad\0text"}, {"panel_x": 100}, {"panel_y": 100},
                      {"panel_x": True, "panel_y": 0}, {"panel_width": 119},
                      {"panel_width": 4001}, {"text_page": -1},
                      {"text_page": 2**32}, {"text_page": True},
                      {"arrow": "false"}, {"target": 0}, {"x": 100}):
            with self.subTest(value=value):
                adapter = Fixed(sc.Decision("settings", 1))
                controller = self.controller(adapter)
                ticket = controller.begin("Settings", self.observation())
                with self.assertRaises(sc.ControllerError):
                    controller.resolve(ticket, value)
                self.assertEqual(adapter.calls, [])


class HTTPTests(Fixtures, unittest.TestCase):
    def native_reply(self):
        return {"model": "fixture", "answers": {"target": {
            "type": "choice", "choice": "settings", "confidence": .95,
            "probabilities": {"settings": .98, sc.NONE: .02}}}}

    def ticket(self):
        return self.controller().begin("Find settings", self.observation())

    def test_typesafe_wire_shape_auth_and_native_choice(self):
        with endpoint(self.native_reply()) as (url, requests), mock.patch.dict(
                os.environ, {"SCREENCUE_TEST_KEY": "fixture-only-not-a-secret"}):
            adapter = sc.ChoiceHTTP(kind="typesafe", endpoint=url, model="jev-test",
                                    key_env="SCREENCUE_TEST_KEY")
            decision = adapter.decide(self.ticket(), 1)
        self.assertEqual(decision.candidate_id, "settings")
        body = requests[0]["body"]
        self.assertEqual(body["model"], "jev-test")
        self.assertEqual(body["questions"]["target"]["type"], "choice")
        self.assertIn(sc.NONE, body["questions"]["target"]["criteria"])
        self.assertEqual(requests[0]["headers"]["Authorization"], "Bearer fixture-only-not-a-secret")

    def test_openai_decisions_wire_shape_and_refusal(self):
        reply = {"answers": [{"name": "target", "type": "refusal"}]}
        with endpoint(reply) as (url, requests), mock.patch.dict(os.environ, {"SCREENCUE_TEST_KEY": "fixture"}):
            adapter = sc.ChoiceHTTP(kind="openai-decisions", endpoint=url, model="gpt-6-luna",
                                   key_env="SCREENCUE_TEST_KEY")
            self.assertTrue(adapter.decide(self.ticket(), 1).refused)
        self.assertIsInstance(requests[0]["body"]["input"], str)
        choices = requests[0]["body"]["questions"][0]["choices"]
        self.assertEqual([c["value"] for c in choices], ["settings", sc.NONE])
        self.assertEqual(adapter.parse({"answers": [{"name": "target", "type": "choice",
            "choice": "settings", "confidence": .9}]}).candidate_id, "settings")

    def test_ollama_local_no_key_and_candidate_limit(self):
        with endpoint(self.native_reply()) as (url, requests):
            adapter = sc.ChoiceHTTP(kind="ollama", endpoint=url, model="nimble", local_only=True)
            self.assertEqual(adapter.decide(self.ticket(), 1).candidate_id, "settings")
        self.assertNotIn("Authorization", requests[0]["headers"])
        candidates = tuple(sc.Candidate(str(i), f"Item {i}", 1, 2, 3, 4) for i in range(26))
        ticket = self.controller().begin("Find one", sc.Observation("frame", candidates, time.monotonic()))
        with self.assertRaises(sc.ControllerError):
            adapter.request(ticket)

    def test_endpoint_credential_redirect_and_local_only_boundaries(self):
        for url in ("http://example.com/v1", "https://user:password@example.com/v1",
                    "https://example.com/v1?key=bad", "file:///tmp/not-http"):
            with self.subTest(url=url), self.assertRaises(sc.ControllerError):
                sc.ChoiceHTTP(kind="ollama", endpoint=url, model="test")
        with self.assertRaises(sc.ControllerError):
            sc.ChoiceHTTP(kind="ollama", endpoint="http://127.0.0.1:11434/v1/systemone", model="clef:cloud", local_only=True)
        with self.assertRaises(sc.ControllerError):
            sc.ChoiceHTTP(kind="ollama", endpoint="https://example.com/v1", model="test", local_only=True)
        with endpoint(self.native_reply()) as (destination, destination_requests):
            with endpoint({}, status=307, headers={"Location": destination}) as (source, _):
                with mock.patch.dict(os.environ, {"SCREENCUE_TEST_KEY": "fixture"}):
                    adapter = sc.ChoiceHTTP(kind="typesafe", endpoint=source, model="test", key_env="SCREENCUE_TEST_KEY")
                    with self.assertRaisesRegex(sc.ControllerError, "HTTP 307"):
                        adapter.decide(self.ticket(), 1)
            self.assertEqual(destination_requests, [])

    def test_missing_key_never_calls_http_and_errors_do_not_echo_provider_body(self):
        with endpoint(b"private provider details", status=401) as (url, requests):
            adapter = sc.ChoiceHTTP(kind="typesafe", endpoint=url, model="test", key_env="SCREENCUE_TEST_MISSING")
            with mock.patch.dict(os.environ, {}, clear=True), self.assertRaises(sc.ControllerError):
                adapter.decide(self.ticket(), 1)
            self.assertEqual(requests, [])
            with mock.patch.dict(os.environ, {"SCREENCUE_TEST_MISSING": "fixture"}):
                with self.assertRaises(sc.ControllerError) as error:
                    adapter.decide(self.ticket(), 1)
            self.assertEqual(str(error.exception), "Decision endpoint returned HTTP 401")

    def test_oversized_or_malformed_response_cannot_draw(self):
        for reply in (b"x" * (sc.MAX_RESPONSE + 1), b"not JSON", {"answers": {"target": {
            "type": "choice", "choice": "settings", "confidence": True}}}):
            with endpoint(reply) as (url, _):
                adapter = sc.ChoiceHTTP(kind="ollama", endpoint=url, model="test")
                with self.assertRaises(sc.ControllerError):
                    adapter.decide(self.ticket(), 1)


class CommandTests(Fixtures, unittest.TestCase):
    def test_local_command_contract_and_unknown_coordinate_fields_rejected(self):
        code = ("import json,sys; r=json.load(sys.stdin); "
                "assert r['version']==1; assert r['candidates'][0]['id']=='settings'; "
                "print(json.dumps({'candidate_id':'settings','confidence':0.95}))")
        adapter = sc.CommandAdapter(command=[sys.executable, "-c", code])
        controller = self.controller(adapter)
        self.assertEqual(controller.resolve(controller.begin("Settings", self.observation())).status, "emitted")
        wrong = sc.CommandAdapter(command=[sys.executable, "-c",
            "print('{\"candidate_id\":\"settings\",\"confidence\":1,\"x\":900}')"])
        with self.assertRaisesRegex(sc.ControllerError, "unsupported fields"):
            wrong.decide(controller.begin("Settings", self.observation()), 1)

    def test_local_command_timeout_nonzero_exit_and_output_budget(self):
        for code, timeout in (("import time; time.sleep(2)", .08),
                              ("raise SystemExit(7)", 1),
                              ("print('x'*70000)", 1)):
            with self.subTest(code=code):
                adapter = sc.CommandAdapter(command=[sys.executable, "-c", code])
                ticket = self.controller().begin("Settings", self.observation())
                with self.assertRaises(sc.ControllerError):
                    adapter.decide(ticket, timeout)

    def test_config_accepts_explicit_adapters_without_network_or_process(self):
        self.assertIsNone(sc.adapter_from_config(None))
        self.assertIsInstance(sc.adapter_from_config({"kind": "command", "command": [sys.executable, "agent.py"]}), sc.CommandAdapter)
        with self.assertRaises(sc.ControllerError):
            sc.adapter_from_config({"kind": "imaginary", "endpoint": "http://localhost", "model": "x"})
        with self.assertRaises(sc.ControllerError):
            sc.adapter_from_config({"kind": "command", "command": "python agent.py"})

    def test_cli_direct_show_emits_only_renderer_commands_and_clears_on_eof(self):
        request = {"command": "show", "goal": "Find the control", "observation_id": "fixture",
            "candidate_id": "settings", "text": "Open settings", "candidates": [{
            "id": "settings", "label": "Settings", "x": -20, "y": 40, "width": 20, "height": 30}]}
        result = subprocess.run([sys.executable, str(ROOT / "controller/screencue_controller.py")],
            input=(json.dumps(request) + "\n").encode(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        messages = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([value["command"] for value in messages], ["show", "clear"])
        self.assertEqual(messages[0]["x"], -20)
        self.assertNotIn(b"Settings", result.stderr)

    def test_cli_accepts_full_panel_text_with_128k_line_budget(self):
        request = {"command": "show", "goal": "😀" * 4000, "observation_id": "fixture",
            "candidate_id": "settings", "title": "Title", "text": "😀" * 16000,
            "panel_x": 100, "panel_y": 100, "panel_width": 600, "text_page": 2,
            "arrow": False, "target": False, "candidates": [{
            "id": "settings", "label": "Settings", "x": 50, "y": 40, "width": 20, "height": 30}]}
        raw = (json.dumps(request, ensure_ascii=False) + "\n").encode()
        self.assertGreater(len(raw), 65536)
        result = subprocess.run([sys.executable, str(ROOT / "controller/screencue_controller.py")],
            input=raw, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=3)
        self.assertEqual(result.returncode, 0, result.stderr)
        shown = json.loads(result.stdout.splitlines()[0])
        self.assertEqual(shown["text"], request["text"])
        self.assertEqual(shown["text_page"], 2)
        self.assertFalse(shown["arrow"])


if __name__ == "__main__":
    unittest.main()
