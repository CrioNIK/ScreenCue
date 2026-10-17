"""Offline process tests for the optional native conversation helper.

No test opens an audio device or contacts a real model service.
"""

from __future__ import annotations

import http.server
import json
import os
from pathlib import Path
import queue
import subprocess
import sys
import tempfile
import threading
import time
import unittest


ROOT = Path(__file__).resolve().parents[1]


def find_helper() -> Path | None:
    explicit = os.environ.get("SCREENCUE_LISTEN_BINARY")
    if explicit:
        candidate = Path(explicit).resolve()
        if not candidate.is_file():
            raise RuntimeError("SCREENCUE_LISTEN_BINARY does not point to a file")
        return candidate
    suffix = ".exe" if sys.platform == "win32" else ""
    for profile in ("debug", "release"):
        candidate = ROOT / "conversation" / "target" / profile / f"screencue-listen{suffix}"
        if candidate.is_file():
            return candidate
    return None


HELPER = find_helper()


@unittest.skipIf(HELPER is None, "Build conversation/Cargo.toml first or set SCREENCUE_LISTEN_BINARY")
class ConversationTests(unittest.TestCase):
    def setUp(self) -> None:
        self.temporary = tempfile.TemporaryDirectory(prefix="screencue-conversation-test-")
        self.root = Path(self.temporary.name)
        self.processes: list[subprocess.Popen] = []

    def tearDown(self) -> None:
        for process in self.processes:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=3)
            for stream in (process.stdin, process.stdout, process.stderr):
                if stream:
                    stream.close()
        self.temporary.cleanup()

    def config(self, value: dict, name: str = "config.json") -> Path:
        path = self.root / name
        path.write_text(json.dumps(value), encoding="utf-8")
        return path

    def start(self, config: dict) -> tuple[subprocess.Popen, queue.Queue]:
        process = subprocess.Popen(
            [str(HELPER), "--config", str(self.config(config)), "--start"],
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            bufsize=1,
        )
        self.processes.append(process)
        events: queue.Queue = queue.Queue()

        def read() -> None:
            for line in process.stdout:
                events.put(json.loads(line))

        threading.Thread(target=read, daemon=True).start()
        self.assertEqual(events.get(timeout=5)["state"], "listening")
        return process, events

    @staticmethod
    def send(process: subprocess.Popen, command: dict) -> None:
        process.stdin.write(json.dumps(command) + "\n")
        process.stdin.flush()

    def adapter(self, body: str) -> dict:
        path = self.root / "adapter.py"
        path.write_text(body, encoding="utf-8")
        return {"kind": "command", "command": [sys.executable, str(path)], "timeout_ms": 1000}

    def test_example_never_overwrites_and_capture_needs_provider(self) -> None:
        path = self.root / "example.json"
        command = [str(HELPER), "--write-example", str(path)]
        self.assertEqual(subprocess.run(command, capture_output=True).returncode, 0)
        original = path.read_bytes()
        self.assertTrue(json.loads(original)["capture"]["system_audio"])
        self.assertNotEqual(subprocess.run(command, capture_output=True).returncode, 0)
        self.assertEqual(path.read_bytes(), original)
        checked = subprocess.run([str(HELPER), "--validate-config", str(path)], capture_output=True, text=True)
        self.assertNotEqual(checked.returncode, 0)
        self.assertIn("transcription provider", checked.stdout)

    def test_validate_does_not_start_command_or_require_credential(self) -> None:
        marker = self.root / "must-not-exist"
        provider = self.adapter(f"from pathlib import Path\nPath({str(marker)!r}).write_text('bad')\n")
        path = self.config({"suggestion": provider})
        result = subprocess.run([str(HELPER), "--validate-config", str(path)], capture_output=True, text=True)
        self.assertEqual(result.returncode, 0)
        self.assertFalse(marker.exists())
        self.assertFalse(json.loads(result.stdout)["provider_contacted"])

    def test_external_transcript_produces_separate_hint_and_stop(self) -> None:
        provider = self.adapter(
            "import json,sys\nrequest=json.load(sys.stdin)\n"
            "assert request['operation']=='suggest'\n"
            "assert 'three pilots' in request['context']\n"
            "print(json.dumps({'text':'We have three pilots.'}))\n"
        )
        process, events = self.start({"context": "We have three pilots", "suggestion": provider})
        self.send(process, {"command": "transcript", "text": "What is your traction?"})
        self.assertEqual(events.get(timeout=3)["event"], "transcript")
        hint = events.get(timeout=3)
        self.assertEqual(hint, {"event": "hint", "title": "Suggested words", "text": "We have three pilots."})
        self.send(process, {"command": "stop"})
        self.assertEqual(events.get(timeout=3)["state"], "stopped")
        self.assertEqual(process.wait(timeout=3), 0)
        self.assertTrue(events.empty())

    def test_stop_cancels_command_and_no_late_hint(self) -> None:
        started = self.root / "started"
        finished = self.root / "finished"
        provider = self.adapter(
            "import json,sys,time\nfrom pathlib import Path\njson.load(sys.stdin)\n"
            f"Path({str(started)!r}).write_text('started')\ntime.sleep(2)\n"
            f"Path({str(finished)!r}).write_text('finished')\nprint(json.dumps({{'text':'late'}}))\n"
        )
        provider["timeout_ms"] = 5000
        process, events = self.start({"suggestion": provider})
        self.send(process, {"command": "transcript", "text": "Hello"})
        self.assertEqual(events.get(timeout=2)["event"], "transcript")
        deadline = time.monotonic() + 2
        while not started.exists() and time.monotonic() < deadline:
            time.sleep(0.01)
        self.assertTrue(started.exists())
        self.send(process, {"command": "stop"})
        self.assertEqual(events.get(timeout=1)["state"], "stopped")
        self.assertEqual(process.wait(timeout=2), 0)
        self.assertFalse(finished.exists())
        self.assertTrue(events.empty())

    def test_http_chat_shape_and_refusal(self) -> None:
        requests = []

        class Handler(http.server.BaseHTTPRequestHandler):
            def do_POST(self) -> None:
                requests.append(json.loads(self.rfile.read(int(self.headers["Content-Length"]))))
                body = json.dumps({"choices": [{"message": {"content": None, "refusal": "No"}}]}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *args) -> None:
                pass

        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        try:
            process, events = self.start({"suggestion": {
                "kind": "openai-compatible", "endpoint": f"http://127.0.0.1:{server.server_port}/chat",
                "model": "fake", "timeout_ms": 1000,
            }})
            self.send(process, {"command": "transcript", "text": "Can you clarify?"})
            self.assertEqual(events.get(timeout=3)["event"], "transcript")
            error = events.get(timeout=3)
            self.assertEqual(error["event"], "error")
            self.assertEqual(error["stage"], "suggestion")
            self.assertIn("declined", error["message"])
            self.assertEqual(requests[0]["model"], "fake")
            self.assertFalse(requests[0]["stream"])
            self.send(process, {"command": "stop"})
            self.assertEqual(events.get(timeout=3)["state"], "stopped")
            process.wait(timeout=3)
        finally:
            server.shutdown()
            server.server_close()

    def test_transcripts_continue_while_suggestion_runs_and_old_answer_is_discarded(self) -> None:
        started = self.root / "started"
        provider = self.adapter(
            "import json,sys,time\nfrom pathlib import Path\nr=json.load(sys.stdin)\n"
            f"Path({str(started)!r}).write_text('started')\n"
            "time.sleep(.3)\nprint(json.dumps({'text': 'latest' if 'Second question' in r['transcript'] else 'stale'}))\n"
        )
        process, events = self.start({"suggestion": provider})
        self.send(process, {"command": "transcript", "text": "First question"})
        self.assertEqual(events.get(timeout=2)["event"], "transcript")
        deadline = time.monotonic() + 2
        while not started.exists() and time.monotonic() < deadline:
            time.sleep(.01)
        self.assertTrue(started.exists())
        self.send(process, {"command": "transcript", "text": "Second question"})
        self.assertEqual(events.get(timeout=.2)["text"], "Second question")
        hint = events.get(timeout=3)
        self.assertEqual(hint["event"], "hint")
        self.assertEqual(hint["text"], "latest")
        self.send(process, {"command": "stop"})
        self.assertEqual(events.get(timeout=2)["state"], "stopped")
        process.wait(timeout=2)

    def test_speaker_roles_are_explicit_and_sources_do_not_imply_identity(self) -> None:
        process, events = self.start({"speaker_roles": {"external:A": "investor"}})
        self.send(process, {"command": "transcript", "segments": [
            {"speaker_id": "A", "text": "What is your runway?"},
            {"speaker_id": "B", "text": "Let me clarify."},
        ]})
        first, second = events.get(timeout=2), events.get(timeout=2)
        self.assertEqual(first["speaker_role"], "investor")
        self.assertIsNone(second["speaker_role"])
        self.assertEqual(second["speaker_id"], "B")
        self.send(process, {"command": "stop"})
        self.assertEqual(events.get(timeout=2)["state"], "stopped")
        process.wait(timeout=2)

    def test_eof_stops_and_unknown_config_never_starts(self) -> None:
        process, events = self.start({})
        process.stdin.close()
        self.assertEqual(events.get(timeout=2)["state"], "stopped")
        self.assertEqual(process.wait(timeout=2), 0)
        result = subprocess.run([str(HELPER), "--config", str(self.config({"api_key": "do-not-echo"}, "bad.json")), "--start"], capture_output=True, text=True)
        self.assertNotEqual(result.returncode, 0)
        self.assertNotIn("do-not-echo", result.stdout)
        self.assertNotIn("listening", result.stdout)


if __name__ == "__main__":
    unittest.main()
