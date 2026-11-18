"""Real daemon/Bridge acceptance for cached authentication, denial and recovery."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import queue
import shutil
import subprocess
import sys
import threading
import time
import traceback

from agent_cli import AgentCli
from run import BINARY, ROOT


class SessionProcess:
    def __init__(self, environment, example=None):
        example = Path(example or os.environ.get("MCPLINK_SESSION_EXAMPLE", ROOT / "target/debug/examples" / ("runtime_session.exe" if os.name == "nt" else "runtime_session")))
        self.process = subprocess.Popen([str(example), str(BINARY)], env=environment, stdin=subprocess.PIPE,
                                        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.responses = queue.Queue()
        self.pending = {}
        self.sequence = 0
        def reader():
            for line in self.process.stdout:
                self.responses.put(json.loads(line))
        threading.Thread(target=reader, daemon=True).start()

    def send(self, method, params=None, **options):
        self.sequence += 1
        self.process.stdin.write(json.dumps({"id": self.sequence, "method": method, "params": params or {}, **options}) + "\n")
        self.process.stdin.flush()
        return self.sequence

    def receive(self, identifier, timeout=40):
        deadline = time.monotonic() + timeout
        while identifier not in self.pending:
            response = self.responses.get(timeout=max(.1, deadline - time.monotonic()))
            self.pending[response["id"]] = response
        return self.pending.pop(identifier)

    def call(self, method, params=None, error=False, **options):
        response = self.receive(self.send(method, params, **options))
        assert ("error" in response) == error, response
        return response.get("error") if error else response["ok"]

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=5)


class Acceptance(AgentCli):
    def run(self):
        self.sessions = []
        session = SessionProcess(self.environment); self.sessions.append(session)
        assert session.call("gateway.stop") == {"stopped": True, "wasRunning": False}
        assert not self.data.exists()
        assert session.call("gateway.status") == {"paused": False, "sessions": 0}
        self.passed("desktop Session starts the supplied CLI, while an absent stop remains offline")

        runtime = json.loads((self.data / "runtime.json").read_text())
        connection = self.cli("connections", "save", "--stdin", value={"name": "session_notes", "definition": {
            "command": sys.executable, "args": [str(ROOT / "tests/fixtures/mcp_server.py"), "--stdio"]}})["id"]
        bridge = self.bridge(connection, "session-cache-entry")
        assert "result" in bridge.call("tools/list")
        key = self.data / "fixture-vault" / hashlib.sha256(b"runtime-key").hexdigest()
        original = key.read_bytes(); key.unlink()
        try:
            identifiers = [session.send("gateway.status") for _ in range(12)]
            assert all("ok" in session.receive(identifier) for identifier in identifiers)
            for _ in range(3):
                assert "result" in bridge.call("tools/list")
            self.passed("concurrent desktop requests and repeated native Bridge calls reuse in-memory authentication")

            started = time.monotonic()
            self.cli("gateway", "status", exit_status=1)
            assert time.monotonic() - started < 5
            assert not key.exists() and json.loads((self.data / "runtime.json").read_text())["pid"] == runtime["pid"]
            self.passed("a fresh client denied its key fails without starting another daemon or replacing credentials")

            denied = SessionProcess(self.environment); self.sessions.append(denied)
            denied.call("gateway.status", error=True)
            key.write_bytes(original)
            denied.call("gateway.status", error=True)
            assert denied.call("gateway.status", retryAuth=True)["paused"] is False
            self.passed("an access denial is latched until an explicit foreground authentication retry")
        finally:
            key.write_bytes(original)

        assert session.call("gateway.stop")["stopped"]
        session.call("mcp.catalog", {"id": connection}, error=True, runningOnly=True)
        time.sleep(2.3)
        assert not (self.data / "runtime.json").exists(), "Bridge catalog observer must not restart a stopped daemon"
        key.write_text("synthetic-replacement-runtime-key")
        assert session.call("gateway.status")["paused"] is False
        after = json.loads((self.data / "runtime.json").read_text())
        assert after["pid"] != runtime["pid"]
        assert "result" in bridge.call("tools/list")
        self.passed("a new daemon instance reauthenticates, and passive catalog observation never starts one")

        marker = self.record / "tree.json"
        tree = self.cli("connections", "save", "--stdin", value={"name": "session_tree", "definition": {
            "command": sys.executable, "args": [str(ROOT / "tests/fixtures/process_tree.py"), "--marker", str(marker)]}})["id"]
        delayed = session.send("mcp.request", {"id": tree, "request": {"method": "tools/call", "params": {
            "name": "process_info", "arguments": {"delay": 60}}}})
        deadline = time.monotonic() + 15
        while not marker.exists() or json.loads(marker.read_text()).get("calls") != 1:
            assert time.monotonic() < deadline
            time.sleep(.05)
        status = session.receive(session.send("gateway.status"), timeout=5)
        assert "ok" in status
        self.cli("gateway", "stop")
        response = session.receive(delayed, timeout=8)
        assert "error" in response and "不要自动重试" in response["error"]
        assert json.loads(marker.read_text())["calls"] == 1
        self.passed("status remains responsive during a call, and interrupted execution is not replayed")

        if sys.platform == "darwin":
            original = ROOT / "target/debug/examples/runtime_session"
            live = self.record / "identity-process"
            replacement = self.record / "identity-replacement"
            shutil.copy2(original, live)
            subprocess.run(["codesign", "--force", "--sign", "-", "--identifier", "io.mcplink.fixture.v1", str(live)], capture_output=True, check=True)
            process = SessionProcess(self.environment, live); self.sessions.append(process)
            assert process.call("process.identity")["valid"]
            shutil.copy2(original, replacement)
            subprocess.run(["codesign", "--force", "--sign", "-", "--identifier", "io.mcplink.fixture.v2", str(replacement)], capture_output=True, check=True)
            os.replace(replacement, live)
            assert "程序已更新" in process.call("process.identity", error=True)
            self.passed("a real macOS process refuses credential access after its signed executable is replaced")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve(); record.mkdir(parents=True, exist_ok=False)
    harness = Acceptance(record)
    conclusion, error = "PASS", None
    try:
        harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception); traceback.print_exc()
    finally:
        for session in getattr(harness, "sessions", []):
            session.close()
        harness.close(False)
    (record / "report.json").write_text(json.dumps({"conclusion": conclusion, "error": error, "scenarios": harness.results,
                                                  "real_keychain_used": False, "real_native_config_modified": False}, ensure_ascii=False, indent=2) + "\n")
    print(conclusion, record)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
