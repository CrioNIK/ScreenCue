"""Opt-in native process tests; no audio, inference, screen capture or external input.

Set SCREENCUE_DESKTOP_BINARY to the built ScreenCue executable to run. The
tests briefly show owned click-through hints and isolate all preferences in a
temporary directory. Ordinary Python discovery skips these desktop tests.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import tempfile
import threading
import time
import unittest


BINARY = os.environ.get("SCREENCUE_DESKTOP_BINARY")


class Client:
    def __init__(self, binary: str, settings: Path):
        self.process = subprocess.Popen(
            [binary, "--json", "--settings-file", str(settings)],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, encoding="utf-8", bufsize=1,
            creationflags=subprocess.CREATE_NO_WINDOW if os.name == "nt" else 0,
        )
        self.responses = queue.Queue()
        self.errors = []

        def read_stdout():
            for line in self.process.stdout:
                try:
                    self.responses.put(json.loads(line))
                except ValueError:
                    self.responses.put({"ok": False, "error": "Non-JSON output"})

        def read_stderr():
            for line in self.process.stderr:
                self.errors.append(line)

        threading.Thread(target=read_stdout, daemon=True).start()
        threading.Thread(target=read_stderr, daemon=True).start()

    def request(self, command):
        self.process.stdin.write(json.dumps(command) + "\n")
        self.process.stdin.flush()
        try:
            response = self.responses.get(timeout=5)
        except queue.Empty as error:
            raise AssertionError("Desktop response timed out: " + "".join(self.errors)) from error
        if not response.get("ok"):
            raise AssertionError(str(response))
        return response

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait(timeout=3)
                raise AssertionError("Desktop process did not exit after JSON EOF")
        for stream in (self.process.stdin, self.process.stdout, self.process.stderr):
            if stream and not stream.closed:
                stream.close()


@unittest.skipUnless(BINARY, "Set SCREENCUE_DESKTOP_BINARY to opt into owned desktop tests")
class DesktopProcessTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="screencue-desktop-test-")
        self.settings = Path(self.temp.name) / "settings.json"
        self.clients = []
        self.save_settings()

    def tearDown(self):
        try:
            for client in self.clients:
                client.close()
        finally:
            self.temp.cleanup()

    def save_settings(self, **changes):
        settings = {"mode": "guidance", "animate": False, "hide_from_capture": False, "panel_width": 300}
        settings.update(changes)
        draft = self.settings.with_suffix(".tmp")
        draft.write_text(json.dumps(settings), encoding="utf-8")
        os.replace(draft, self.settings)

    def client(self):
        client = Client(str(Path(BINARY).resolve()), self.settings)
        self.clients.append(client)
        return client

    def show(self, client, **changes):
        monitor = client.request({"command": "displays"})["displays"][0]
        command = {
            "command": "show", "x": monitor["x"] + monitor["width"] // 2,
            "y": monitor["y"] + monitor["height"] // 2,
            "title": "ScreenCue lifecycle test", "text": "Owned test hint",
            "arrow": False, "target": False, "follow_window": False,
            "animate": False, "color": "#EF232D", "duration_ms": 0,
        }
        command.update(changes)
        command = {key: value for key, value in command.items() if value is not None}
        response = client.request(command)
        self.assertTrue(response["visible"])
        self.assertEqual(response["color_source"], "manual")
        return response

    def wait_status(self, client, predicate, timeout=2):
        deadline = time.monotonic() + timeout
        latest = None
        while time.monotonic() < deadline:
            latest = client.request({"command": "status"})
            if predicate(latest):
                return latest
            time.sleep(.03)
        self.fail(f"Native state did not converge: {latest}")

    def test_preferences_propagate_to_visible_connections_and_next_request(self):
        first, second = self.client(), self.client()
        for client in (first, second):
            self.show(client)
        self.save_settings(mode="conversation", hide_from_capture=True)
        for client in (first, second):
            status = self.wait_status(client, lambda item: item["capture_privacy"]["requested"])
            self.assertEqual(status["conversation_state"], "Stopped")
            self.assertFalse(status["capture_privacy"]["capture_guaranteed"])
            if os.name == "nt":
                self.assertTrue(status["capture_privacy"]["applied"])
                self.assertTrue(status["capture_privacy"]["verified"])
            self.show(client, arrow=None, target=None, animate=True)
            status = client.request({"command": "status"})
            self.assertFalse(status["hint"]["arrow"])
            self.assertFalse(status["hint"]["target"])
            self.assertFalse(status["hint"]["animate"])
            self.show(client, arrow=True, target=True, animate=False)
            explicit = client.request({"command": "status"})
            self.assertTrue(explicit["hint"]["arrow"])
            self.assertTrue(explicit["hint"]["target"])
        self.save_settings()
        for client in (first, second):
            status = self.wait_status(client, lambda item: not item["capture_privacy"]["requested"])
            self.assertFalse(status["capture_privacy"]["applied"])

    def test_dismiss_token_clears_every_connection_but_allows_explicit_reshow(self):
        first, second = self.client(), self.client()
        for client in (first, second):
            self.show(client)
        self.settings.with_suffix(".dismiss").write_text("owned-test-dismiss-1", encoding="utf-8")
        for client in (first, second):
            status = self.wait_status(client, lambda item: not item["visible"])
            self.assertIsNone(status["hint"])
            self.assertFalse(status["shortcut_active"])
        self.show(first)
        time.sleep(.35)
        self.assertTrue(first.request({"command": "status"})["visible"])
        self.assertFalse(second.request({"command": "status"})["visible"])
        self.settings.with_suffix(".dismiss").write_text("owned-test-dismiss-2", encoding="utf-8")
        self.wait_status(first, lambda item: not item["visible"])

    def test_idle_connection_reloads_privacy_before_its_first_visible_hint(self):
        client = self.client()
        self.assertFalse(client.request({"command": "status"})["visible"])
        self.save_settings(hide_from_capture=True, mode="conversation")
        shown = self.show(client)
        self.assertTrue(shown["capture_privacy"]["requested"])
        client.request({"command": "clear"})
        self.assertFalse(client.request({"command": "status"})["visible"])
        client.close()
        self.assertEqual(client.process.returncode, 0)

    def test_explicit_clear_broadcasts_to_other_agent_connections(self):
        first, second = self.client(), self.client()
        for client in (first, second):
            self.show(client)
        first.request({"command": "clear"})
        self.assertFalse(first.request({"command": "status"})["visible"])
        self.wait_status(second, lambda item: not item["visible"])

    @unittest.skipUnless(os.name == "nt", "Owned native window lifecycle test requires Windows")
    def test_settings_start_stop_restart_and_close_reap_synthetic_helper(self):
        self._settings_helper_lifecycle(managed=False)

    @unittest.skipUnless(os.name == "nt", "Owned native window lifecycle test requires Windows")
    def test_managed_connections_route_independent_environment_keys_without_audio(self):
        self._settings_helper_lifecycle(managed=True)

    def _settings_helper_lifecycle(self, managed):
        import ctypes
        from ctypes import wintypes

        user32, kernel32 = ctypes.WinDLL("user32", use_last_error=True), ctypes.WinDLL("kernel32", use_last_error=True)
        callback_type = ctypes.WINFUNCTYPE(wintypes.BOOL, wintypes.HWND, wintypes.LPARAM)
        user32.EnumWindows.argtypes = [callback_type, wintypes.LPARAM]
        user32.GetWindowThreadProcessId.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.DWORD)]
        user32.GetWindowTextW.argtypes = [wintypes.HWND, wintypes.LPWSTR, ctypes.c_int]
        user32.GetClientRect.argtypes = [wintypes.HWND, ctypes.POINTER(wintypes.RECT)]
        user32.IsWindowVisible.argtypes = [wintypes.HWND]
        user32.PostMessageW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM, wintypes.LPARAM]
        user32.SendMessageTimeoutW.argtypes = [wintypes.HWND, wintypes.UINT, wintypes.WPARAM,
            wintypes.LPARAM, wintypes.UINT, wintypes.UINT, ctypes.POINTER(ctypes.c_size_t)]
        user32.SendMessageTimeoutW.restype = wintypes.LPARAM
        kernel32.OpenProcess.argtypes = [wintypes.DWORD, wintypes.BOOL, wintypes.DWORD]
        kernel32.OpenProcess.restype = wintypes.HANDLE
        kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
        kernel32.CloseHandle.argtypes = [wintypes.HANDLE]

        def find_window(pid, title):
            result = []
            @callback_type
            def inspect(window, _):
                owner = wintypes.DWORD()
                user32.GetWindowThreadProcessId(window, ctypes.byref(owner))
                if owner.value == pid:
                    text = ctypes.create_unicode_buffer(256)
                    user32.GetWindowTextW(window, text, 256)
                    if text.value == title:
                        result.append(window)
                        return False
                return True
            user32.EnumWindows(inspect, 0)
            return result[0] if result else None

        def wait_for(predicate, message, timeout=5):
            deadline = time.monotonic() + timeout
            while time.monotonic() < deadline:
                result = predicate()
                if result:
                    return result
                time.sleep(.02)
            self.fail(message)

        def live(pid):
            handle = kernel32.OpenProcess(0x1000, False, pid)
            if not handle:
                return False
            try:
                code = wintypes.DWORD()
                return bool(kernel32.GetExitCodeProcess(handle, ctypes.byref(code))) and code.value == 259
            finally:
                kernel32.CloseHandle(handle)

        def click(window, x, y):
            bounds = wintypes.RECT()
            self.assertTrue(user32.GetClientRect(window, ctypes.byref(bounds)))
            width, height = bounds.right, bounds.bottom
            scale = min(width / 760, height / 730)
            physical_x = round((width - 760 * scale) / 2 + x * scale)
            physical_y = round((height - 730 * scale) / 2 + y * scale)
            location = physical_x | (physical_y << 16)
            result = ctypes.c_size_t()
            for message, buttons in ((0x200, 0), (0x201, 1), (0x202, 0)):
                self.assertTrue(user32.SendMessageTimeoutW(window, message, buttons, location, 2, 3000, ctypes.byref(result)))

        # This fixture is deliberately not the production audio helper. It never
        # opens devices or invokes models, and its last session ignores Stop so
        # application shutdown must execute the bounded fallback kill.
        application = Path(self.temp.name) / "owned-app"
        application.mkdir()
        binary = application / "screencue.exe"
        shutil.copy2(BINARY, binary)
        (application / "screencue-listen.exe").write_bytes(b"owned fixture placeholder; never executed")
        (application / "controller").mkdir()
        script = application / "controller/conversation_orchestrator.py"
        script.write_text(
            "import json,os,sys,time\nfrom pathlib import Path\n"
            "config=json.loads(Path(sys.argv[sys.argv.index('--config')+1]).read_text())\n"
            "root=Path(__file__).resolve().parents[1]\npid=os.getpid()\n"
            "if config.get('fast'):\n"
            " assert os.environ[config['fast']['key_env']]=='synthetic-fast-value'\n"
            " assert os.environ[config['planner']['key_env']]=='synthetic-planner-value'\n"
            " assert config.get('helper_config') is None\n"
            "marker=root/f'helper-{pid}.json'\n"
            "marker.write_text(json.dumps({'pid':pid,'state':'started'}))\n"
            "def emit(value):\n print(json.dumps(value),flush=True)\n"
            "emit({'event':'status','state':'ready'})\n"
            "emit({'event':'hint','title':'Owned lifecycle test','text':'Synthetic helper; no audio or model.'})\n"
            "for line in sys.stdin:\n"
            " if json.loads(line).get('command')=='stop': break\n"
            "emit({'event':'hint','title':'Stale fixture','text':'This late answer must stay hidden.'})\n"
            "marker.write_text(json.dumps({'pid':pid,'state':'late_sent'}))\n"
            "if config.get('ignore_stop') or (root/'ignore-stop.flag').exists(): time.sleep(30)\n"
            "emit({'event':'status','state':'stopped'})\n",
            encoding="utf-8",
        )
        config = application / "fixture.json"
        def save_config(ignore_stop=False):
            if managed:
                if ignore_stop:
                    (application / "ignore-stop.flag").touch()
                return
            config.write_text(json.dumps({
                "engine": "conversation-orchestrator", "python_executable": sys.executable,
                "ignore_stop": ignore_stop,
            }), encoding="utf-8")
        save_config()
        environment = os.environ.copy()
        if managed:
            environment.update(SCREENCUE_TEST_FAST_INPUT_KEY="synthetic-fast-value",
                               SCREENCUE_TEST_PLANNER_INPUT_KEY="synthetic-planner-value")
            def api(auth, key_env=""):
                return {"kind": "openai-compatible", "endpoint": "http://127.0.0.1:1/unused",
                        "model": "synthetic", "auth": auth, "key_env": key_env}
            self.save_settings(mode="conversation", connections={
                "enabled": True, "python_executable": sys.executable,
                "microphone": False, "system_audio": False,
                # An unused audio profile must not require a saved key.
                "transcription": api("vault"),
                "fast": api("environment", "SCREENCUE_TEST_FAST_INPUT_KEY"),
                "planner": api("environment", "SCREENCUE_TEST_PLANNER_INPUT_KEY"),
            })
        else:
            self.save_settings(mode="conversation", audio_config=str(config))
        process = subprocess.Popen(
            [str(binary), "--app", "--settings-file", str(self.settings)],
            stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE,
            creationflags=subprocess.CREATE_NO_WINDOW,
            env=environment,
        )
        def markers():
            return {int(path.stem.split("-")[1]) for path in application.glob("helper-*.json")}
        def new_helper(existing):
            return wait_for(lambda: next(iter(markers() - existing), None), "Start did not launch the synthetic helper")
        try:
            window = wait_for(lambda: find_window(process.pid, "ScreenCue"), "Settings window did not open")
            click(window, 80, 224)
            click(window, 320, 650)
            first = new_helper(set())
            if managed:
                runtime = list(Path(self.temp.name).glob(".screencue-runtime-*/*.json"))
                self.assertTrue(runtime)
                for path in runtime:
                    text = path.read_text(encoding="utf-8")
                    self.assertNotIn("synthetic-fast-value", text)
                    self.assertNotIn("synthetic-planner-value", text)
            overlay = wait_for(lambda: find_window(process.pid, "ScreenCue Overlay"), "Owned overlay was not created")
            wait_for(lambda: user32.IsWindowVisible(overlay), "Helper hint was not displayed")
            click(window, 590, 650)
            wait_for(lambda: not live(first), "Stop did not reap the synthetic helper")
            wait_for(lambda: not user32.IsWindowVisible(overlay), "Stop did not hide its hint")
            self.assertEqual(json.loads((application / f"helper-{first}.json").read_text())["state"], "late_sent")
            time.sleep(.2)
            self.assertFalse(user32.IsWindowVisible(overlay), "A late helper answer resurrected the overlay")
            click(window, 320, 650)
            second = new_helper({first})
            wait_for(lambda: user32.IsWindowVisible(overlay), "Restarted session did not display its hint")
            save_config(ignore_stop=True)
            click(window, 320, 650)
            third = new_helper({first, second})
            wait_for(lambda: not live(second), "Starting again left the previous helper alive")
            wait_for(lambda: user32.IsWindowVisible(overlay), "Replacement session did not display its hint")
            self.assertTrue(user32.PostMessageW(window, 0x10, 0, 0))
            self.assertEqual(process.wait(timeout=5), 0)
            self.assertFalse(live(third), "App shutdown abandoned a helper that ignored Stop")
        finally:
            if process.poll() is None:
                window = find_window(process.pid, "ScreenCue")
                if window:
                    user32.PostMessageW(window, 0x10, 0, 0)
                try:
                    process.wait(timeout=3)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=2)
            process.stderr.close()
            for pid in markers():
                if live(pid):
                    handle = kernel32.OpenProcess(1, False, pid)
                    if handle:
                        kernel32.TerminateProcess(handle, 1)
                        kernel32.CloseHandle(handle)


if __name__ == "__main__":
    unittest.main()
