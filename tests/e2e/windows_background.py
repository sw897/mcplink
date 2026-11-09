"""Exercise persisted pause, console-free STDIO and process-tree cleanup through the CLI."""
import argparse
import ctypes
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import sys
import time
import traceback

from agent_cli import AgentCli
from run import BINARY, ROOT


def exited(pid):
    kernel = ctypes.windll.kernel32
    kernel.OpenProcess.restype = ctypes.c_void_p
    kernel.WaitForSingleObject.argtypes = [ctypes.c_void_p, ctypes.c_uint32]
    kernel.CloseHandle.argtypes = [ctypes.c_void_p]
    handle = kernel.OpenProcess(0x00100000, False, pid)
    if not handle:
        return True
    try:
        return kernel.WaitForSingleObject(handle, 0) == 0
    finally:
        kernel.CloseHandle(handle)


class WindowsBackground(AgentCli):
    def restart(self):
        previous = json.loads((self.data / "runtime.json").read_text())
        os.kill(previous["pid"], signal.SIGTERM)
        deadline = time.monotonic() + 5
        while not exited(previous["pid"]):
            assert time.monotonic() < deadline
            time.sleep(.05)
        state = self.cli("gateway", "status")
        current = json.loads((self.data / "runtime.json").read_text())
        assert current["instance"] != previous["instance"]
        return state

    def run(self):
        assert self.cli("gateway", "status")["paused"] is False
        assert self.cli("gateway", "pause")["paused"] is True
        assert self.restart()["paused"] is True
        self.passed("pause is committed before acknowledgement and survives forced daemon restart")

        connection = self.cli("connections", "save", "--stdin", value={
            "name": "windows_runtime", "definition": {"command": sys.executable,
            "args": [str(ROOT / "tests/fixtures/windows_stdio.py")]}})["id"]
        denied = self.cli("mcp", "describe", connection, exit_status=1)
        assert "暂停" in denied["error"]["message"]
        self.cli("gateway", "resume")
        assert self.restart()["paused"] is False
        self.passed("resume persists and paused shared requests stay blocked across restarts")

        probe = self.cli("connections", "probe", connection)
        assert probe["connection"]["status"] == "ready", probe
        bridge = self.bridge(connection, "windows-background-check")
        result = bridge.call("tools/call", {"name": "windows_runtime", "arguments": {}})
        process = json.loads(result["result"]["content"][0]["text"])
        assert process["consoleWindow"] == 0, process
        self.passed("an upstream console executable starts without a console window")

        assert self.restart()["paused"] is False
        deadline = time.monotonic() + 5
        while not all(exited(process[key]) for key in ("pid", "descendant")):
            assert time.monotonic() < deadline, process
            time.sleep(.05)
        self.passed("forced daemon exit terminates the upstream process and its descendant")

        result = bridge.call("tools/call", {"name": "windows_runtime", "arguments": {}})
        recovered = json.loads(result["result"]["content"][0]["text"])
        assert recovered["pid"] != process["pid"] and recovered["consoleWindow"] == 0
        self.passed("an existing Bridge reconnects to the restarted daemon without replaying a failed call")

        self.cli("gateway", "pause")
        deadline = time.monotonic() + 5
        while not all(exited(recovered[key]) for key in ("pid", "descendant")):
            assert time.monotonic() < deadline
            time.sleep(.05)
        self.passed("pausing closes the full upstream process tree")
        self.cli("gateway", "resume")
        return {"connection": connection, "upstream": process, "recovered": recovered}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    assert os.name == "nt", "this suite must run on Windows"
    harness = WindowsBackground(record)
    conclusion, error, ids = "PASS", None, {}
    try:
        ids = harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(False)
    report = {"conclusion": conclusion, "error": error, "executed_at": datetime.now(timezone.utc).isoformat(),
              "os": platform.platform(), "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
              "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "scenarios": harness.results, "commands": harness.commands, "ids": ids,
              "real_native_mcp_config_modified": False}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
