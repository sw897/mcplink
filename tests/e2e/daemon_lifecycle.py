"""Public CLI acceptance for shutdown, process ownership and on-demand restart."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import subprocess
import sys
import time
import traceback

from agent_cli import AgentCli
from run import BINARY, ROOT


def exited(pid):
    if os.name == "nt":
        from windows_background import exited as windows_exited
        return windows_exited(pid)
    try:
        os.kill(pid, 0)
    except ProcessLookupError:
        return True
    status = subprocess.run(["/bin/ps", "-o", "stat=", "-p", str(pid)], capture_output=True, text=True).stdout.strip()
    return not status or status.startswith("Z")


def wait_until(predicate, seconds=30):
    deadline = time.monotonic() + seconds
    while not predicate():
        assert time.monotonic() < deadline, "lifecycle condition timed out"
        time.sleep(.05)


class Lifecycle(AgentCli):
    def save_tree(self, name, *options, command=None):
        marker = self.record / (name + ".json")
        connection = self.cli("connections", "save", "--stdin", value={"name": name, "definition": {
            "command": command or sys.executable,
            "args": [str(ROOT / "tests/fixtures/process_tree.py"), "--marker", str(marker), *options]}})["id"]
        return connection, marker

    def stop(self, observation=None):
        runtime = json.loads((self.data / "runtime.json").read_text())
        result = self.cli("gateway", "stop")
        assert result["stopped"] and result["wasRunning"], result
        wait_until(lambda: exited(runtime["pid"]))
        assert not (self.data / "runtime.json").exists()
        if observation:
            wait_until(lambda: all(exited(observation[key]) for key in ("pid", "descendant")))

    def run(self):
        result = self.cli("gateway", "stop")
        assert result == {"stopped": True, "wasRunning": False}
        assert not self.data.exists()
        self.passed("stopping an absent daemon is offline and creates no data directory or credentials")

        assert not self.cli("gateway", "status")["paused"]
        connection, marker = self.save_tree("normal_tree")
        bridge = self.bridge(connection, "lifecycle-native-entry")
        result = bridge.call("tools/call", {"name": "process_info", "arguments": {}})
        observation = json.loads(result["result"]["content"][0]["text"])
        if sys.platform == "darwin":
            assert observation["group"] == observation["pid"]
        self.stop(observation)
        assert not self.cli("gateway", "stop")["wasRunning"]
        self.passed("explicit stop waits for the owned upstream tree and daemon to exit and removes runtime metadata")

        result = bridge.call("tools/call", {"name": "process_info", "arguments": {}})
        restored = json.loads(result["result"]["content"][0]["text"])
        assert restored["pid"] != observation["pid"]
        assert not self.cli("gateway", "status")["paused"]
        self.passed("a subsequent Agent request restarts the stopped daemon and reuses its unchanged connection entry")

        self.cli("gateway", "pause")
        self.stop()
        assert self.cli("gateway", "status")["paused"]
        self.cli("gateway", "resume")
        self.passed("exit cleanup preserves both running and paused persisted preferences")

        slow, slow_marker = self.save_tree("inflight_tree", "--stubborn")
        slow_bridge = self.bridge(slow, "lifecycle-inflight")
        with ThreadPoolExecutor(max_workers=1) as executor:
            call = executor.submit(slow_bridge.call, "tools/call", {"name": "process_info", "arguments": {"delay": 60}})
            wait_until(lambda: json.loads(slow_marker.read_text())["calls"] == 1)
            current = json.loads(slow_marker.read_text())
            self.stop(current)
            answer = json.dumps(call.result(timeout=5), ensure_ascii=False)
        assert "error" in answer and "不要自动重试" in answer, answer
        assert json.loads(slow_marker.read_text())["calls"] == 1
        self.passed("shutdown interrupts an in-flight call once, reports uncertain execution and kills a stubborn process tree")

        hanging, hanging_marker = self.save_tree("startup_tree", "--hang-initialize", "--stubborn")
        process = subprocess.Popen([str(BINARY), "connections", "probe", hanging], env=self.environment,
            stdin=subprocess.DEVNULL, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
        try:
            wait_until(hanging_marker.exists)
            self.stop(json.loads(hanging_marker.read_text()))
            stdout, stderr = process.communicate(timeout=5)
            assert process.returncode != 0 and not stderr, (stdout, stderr)
        finally:
            if process.poll() is None:
                process.kill()
                process.wait(timeout=5)
        self.passed("stopping during MCP initialization cancels the startup and cleans its entire process group")

        if sys.platform == "darwin":
            self.mac_cases()

    def mac_cases(self):
        import plistlib
        # A Finder-style minimal environment must not lose user-installed MCP commands.
        self.environment["PATH"] = "/usr/bin:/bin"
        launcher = self.home / ".local/bin/lifecycle-python"
        launcher.parent.mkdir(parents=True, exist_ok=True)
        import shlex
        launcher.write_text("#!/bin/sh\nexec " + shlex.quote(sys.executable) + ' "$@"\n')
        launcher.chmod(0o755)
        connection, marker = self.save_tree("finder_tree", command=launcher.name)
        bridge = self.bridge(connection, "finder-minimal-path")
        assert "result" in bridge.call("tools/call", {"name": "process_info", "arguments": {}})
        observation = json.loads(marker.read_text())
        assert observation["group"] == observation["pid"]
        runtime = json.loads((self.data / "runtime.json").read_text())
        assert os.getpgid(runtime["pid"]) == runtime["pid"]
        os.kill(runtime["pid"], signal.SIGTERM)
        wait_until(lambda: exited(runtime["pid"]))
        wait_until(lambda: all(exited(observation[key]) for key in ("pid", "descendant")))
        assert not (self.data / "runtime.json").exists()
        self.passed("macOS Finder-style PATH works and SIGTERM cleans independent daemon/upstream process groups")

        for launcher in self.launchers.values():
            launcher.unlink(missing_ok=True)
        bundle = self.home / "Applications/Codex.app"
        (bundle / "Contents/MacOS").mkdir(parents=True)
        assert not next(a for a in self.cli("status")["agents"] if a["id"] == "codex")["installed"]
        (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable": "fixture-codex"}, fmt=plistlib.FMT_BINARY))
        binary = bundle / "Contents/MacOS/fixture-codex"
        binary.touch()
        assert next(a for a in self.cli("status")["agents"] if a["id"] == "codex")["installed"]
        binary.unlink()
        assert not next(a for a in self.cli("status")["agents"] if a["id"] == "codex")["installed"]
        self.passed("macOS installation detection reads binary plists and requires the actual bundle executable")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = Lifecycle(record)
    conclusion, error = "PASS", None
    try:
        harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(False)
    report = {"conclusion": conclusion, "error": error, "executed_at": datetime.now(timezone.utc).isoformat(),
              "os": platform.platform(), "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
              "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "scenarios": harness.results, "commands": harness.commands,
              "real_native_mcp_config_modified": False, "macos_cases_executed": sys.platform == "darwin"}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
