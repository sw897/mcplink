"""Run installed Codex and OpenCode against one shared MCP connection, with isolated configs.

Codex uses its native app-server tool-call API. OpenCode uses its native agent loop
with a deterministic loopback-only model fixture; no paid model/provider is called.
"""
import argparse
import base64
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import queue
import socket
import subprocess
import threading
import time
import traceback
import urllib.error
import urllib.request

from tool_calls import ToolCalls
from run import BINARY


def port():
    with socket.socket() as listener:
        listener.bind(("127.0.0.1", 0))
        return listener.getsockname()[1]


class ModelFixture(BaseHTTPRequestHandler):
    def log_message(self, *args):
        pass

    def do_POST(self):
        body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
        names = [tool["function"]["name"] for tool in body.get("tools", [])]
        observation = {"path": self.path, "tools": names,
            "execute": next((tool["function"] for tool in body.get("tools", []) if tool["function"]["name"] == "execute"), None),
            "mcp_reference": [line for message in body.get("messages", []) if isinstance(message.get("content"), str)
                              for line in message["content"].splitlines() if "shared_acceptance" in line]}
        self.server.observations.append(observation)
        (self.server.record / "model-observations.json").write_text(json.dumps(self.server.observations, ensure_ascii=False, indent=2))
        messages = body.get("messages", [])
        # Request exactly one read-only MCP tool, then finish after the native client supplies its result.
        last_user = max((i for i, m in enumerate(messages) if m.get("role") == "user"), default=-1)
        completed = any(m.get("role") == "tool" for m in messages[last_user + 1:])
        tool = next((name for name in names if "read_document" in name), None)
        arguments = {"id": "weekly-fixture"}
        if not tool and "execute" in names and any("tools.shared_acceptance.read_document(" in line for line in observation["mcp_reference"]):
            tool = "execute"
            arguments = {"code": 'return await tools.shared_acceptance.read_document({id:"weekly-fixture"});'}
        if not completed and tool:
            delta = {"role": "assistant", "tool_calls": [{"index": 0, "id": "call_native_fixture",
                "type": "function", "function": {"name": tool, "arguments": json.dumps(arguments)}}]}
            reason = "tool_calls"
        else:
            delta = {"role": "assistant", "content": "Synthetic MCP read completed." if completed else "No MCP read tool was advertised."}
            reason = "stop"
        if body.get("stream"):
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.end_headers()
            for value in [delta, {}]:
                event = {"id": "chatcmpl-fixture", "object": "chat.completion.chunk", "created": 1,
                         "model": "fixture", "choices": [{"index": 0, "delta": value,
                         "finish_reason": reason if not value else None}]}
                self.wfile.write(("data: " + json.dumps(event) + "\n\n").encode())
            self.wfile.write(b"data: [DONE]\n\n")
        else:
            result = {"id": "chatcmpl-fixture", "object": "chat.completion", "created": 1,
                      "model": "fixture", "choices": [{"index": 0, "message": delta, "finish_reason": reason}],
                      "usage": {"prompt_tokens": 1, "completion_tokens": 1, "total_tokens": 2}}
            encoded = json.dumps(result).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
        self.wfile.flush()


class AppServer:
    def __init__(self, executable, environment, project, record):
        self.log = (record / "codex-native.log").open("w")
        self.process = subprocess.Popen([str(executable), "app-server", "--listen", "stdio://"],
            env=environment, cwd=project, stdin=subprocess.PIPE, stdout=subprocess.PIPE,
            stderr=self.log, text=True, bufsize=1, creationflags=subprocess.CREATE_NO_WINDOW)
        self.messages = queue.Queue()
        self.sequence = 0
        self.notifications = []
        def reader():
            for line in self.process.stdout:
                self.messages.put(json.loads(line))
            self.messages.put(None)
        threading.Thread(target=reader, daemon=True).start()
        self.call("initialize", {"clientInfo": {"name": "mcplink_native_acceptance", "version": "1.0.0"},
                                  "capabilities": {"experimentalApi": True}})
        self.send({"method": "initialized"})

    def send(self, value):
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", **value}) + "\n")
        self.process.stdin.flush()

    def call(self, method, params):
        self.sequence += 1
        self.send({"id": self.sequence, "method": method, "params": params})
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            value = self.messages.get(timeout=max(.1, deadline - time.monotonic()))
            assert value is not None, "native Codex app-server closed"
            if value.get("id") == self.sequence:
                return value
            self.notifications.append(value)
        raise AssertionError("native Codex app-server timed out")

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=8)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=8)
        self.log.close()


class NativeSharedAgents(ToolCalls):
    def run_native(self, codex, opencode):
        self.native_codex = None
        self.native_opencode = None
        self.model = ThreadingHTTPServer(("127.0.0.1", 0), ModelFixture)
        self.model.observations = []
        self.model.record = self.record
        threading.Thread(target=self.model.serve_forever, daemon=True).start()
        project = self.record / "project"
        project.mkdir()
        subprocess.run(["git", "init", "--quiet", str(project)], check=True, capture_output=True)
        codex_config = self.home / ".codex/config.toml"
        codex_config.write_text('model = "fixture"\nmodel_provider = "acceptance"\n'
            '[analytics]\nenabled = false\n'
            '[model_providers.acceptance]\nname = "Local acceptance"\n'
            'base_url = "http://127.0.0.1:9/v1"\nwire_api = "responses"\nrequires_openai_auth = false\n')
        config = self.home / ".config/opencode/opencode.json"
        config.parent.mkdir(parents=True, exist_ok=True)
        config.write_text(json.dumps({"model": "acceptance/fixture", "update": "disable", "share": "disabled",
            "providers": {"acceptance": {"name": "Local acceptance", "package": "@ai-sdk/openai-compatible",
                "settings": {"baseURL": f"http://127.0.0.1:{self.model.server_port}/v1", "apiKey": "synthetic-local-only"},
                "models": {"fixture": {"name": "Fixture", "limit": {"context": 32000, "output": 1024},
                    "capabilities": {"tools": True, "input": ["text"], "output": ["text"]}}}}},
            "mcp": {"servers": {}}}))
        connection = self.save("shared_acceptance", url=f"http://127.0.0.1:{self.port}/mcp-session")
        assert self.cli("connections", "probe", connection)["connection"]["status"] == "ready"
        # The required preflight probe uses and closes a temporary, unbound session.
        preflight_sessions = len(self.stats()["sessionVersions"])
        sources = self.cli("sources", "list")
        opencode_source = next(source["id"] for source in sources if Path(source["path"]) == config)
        plan = self.cli("changes", "preview", "--connection", connection, "--target", "codex:user",
                        "--target", opencode_source, "--mode", "shared")
        applied = self.cli("changes", "apply", plan["planId"])
        assert all(row["status"] == "applied" for row in applied["results"])
        (self.record / "native-config-plan.json").write_text(json.dumps(plan, ensure_ascii=False, indent=2))
        assert "synthetic-fixture-authorization" not in codex_config.read_text() + config.read_text()
        self.passed("one managed credential is referenced by both native configurations without exporting its value")

        environment = self.environment.copy()
        for name in ["OPENCODE_CONFIG", "OPENCODE_CONFIG_CONTENT", "OPENCODE_SERVER", "OPENCODE_SERVER_PASSWORD",
                     "OPENAI_API_KEY", "OPENAI_BASE_URL", "CODEX_API_KEY"]:
            environment.pop(name, None)
        environment["OPENCODE_PASSWORD"] = "synthetic-native-acceptance"
        self.native_codex = AppServer(codex, environment, project, self.record)
        started = self.native_codex.call("thread/start", {"cwd": str(project), "model": "fixture",
            "approvalPolicy": "never", "sandbox": "workspace-write", "ephemeral": True})
        assert "result" in started, started
        thread = started["result"]["thread"]["id"]
        listing = self.native_codex.call("mcpServerStatus/list", {"threadId": thread, "serverName": "shared_acceptance"})
        (self.record / "codex-mcp-status.json").write_text(json.dumps(listing, ensure_ascii=False, indent=2))
        assert "read_document" in json.dumps(listing), listing
        shared_sessions = len(self.stats()["sessionVersions"])
        assert shared_sessions == preflight_sessions + 1
        self.passed("installed Codex discovers the generated Bridge with its native app-server MCP client")

        self.native_port = port()
        self.native_log = (self.record / "opencode-native.log").open("w")
        self.native_opencode = subprocess.Popen([str(opencode), "serve", "--hostname", "127.0.0.1", "--port", str(self.native_port)],
            env=environment, cwd=project, stdout=self.native_log, stderr=self.native_log, creationflags=subprocess.CREATE_NO_WINDOW)
        deadline = time.monotonic() + 45
        while True:
            try:
                status = self.http("/api/mcp")
                (self.record / "opencode-mcp-status.json").write_text(json.dumps(status, ensure_ascii=False, indent=2))
                if any(row["name"] == "shared_acceptance" and row["status"]["status"] == "connected" for row in status["data"]):
                    break
                if any(row["name"] == "shared_acceptance" for row in status["data"]):
                    connected = self.http("/api/experimental/mcp/shared_acceptance/connect", {})
                    (self.record / "opencode-connect.json").write_text(json.dumps(connected, ensure_ascii=False, indent=2))
            except (urllib.error.URLError, json.JSONDecodeError, KeyError):
                pass
            assert time.monotonic() < deadline, "native OpenCode did not connect; inspect its isolated log"
            time.sleep(.25)
        (self.record / "opencode-mcp-status.json").write_text(json.dumps(status, ensure_ascii=False, indent=2))
        self.passed("installed OpenCode connects the same Bridge through its native v2 MCP client")

        def codex_read():
            return self.native_codex.call("mcpServer/tool/call", {"threadId": thread, "server": "shared_acceptance",
                "tool": "read_document", "arguments": {"id": "weekly-fixture"}})

        def opencode_read():
            session = self.http("/api/session", {"model": {"providerID": "acceptance", "id": "fixture"},
                "title": "Local MCP acceptance", "permissions": [{"action": "*", "resource": "*", "effect": "allow"}]})
            (self.record / "opencode-session.json").write_text(json.dumps(session, ensure_ascii=False, indent=2))
            session_id = session["id"] if "id" in session else session["data"]["id"]
            self.http(f"/api/session/{session_id}/prompt", {"text": "Read weekly-fixture using the shared MCP read_document tool."})
            deadline = time.monotonic() + 60
            while True:
                messages = self.http(f"/api/session/{session_id}/message")
                text = json.dumps(messages, ensure_ascii=False)
                if "fixture-workspace" in text:
                    return messages
                assert time.monotonic() < deadline, "native OpenCode did not return the MCP result"
                (self.record / "opencode-messages.json").write_text(json.dumps(messages, ensure_ascii=False, indent=2))
                time.sleep(.25)

        before = self.stats()["calls"]
        with ThreadPoolExecutor(max_workers=2) as executor:
            a, b = executor.submit(codex_read), executor.submit(opencode_read)
            responses = {"codex": a.result(), "opencode": b.result()}
        for result in responses.values():
            assert "fixture-workspace" in json.dumps(result, ensure_ascii=False), result
        assert self.stats()["calls"] == before + 2
        assert len(self.stats()["sessionVersions"]) == shared_sessions, self.stats()
        assert self.cli("gateway", "status")["sessions"] == 1
        (self.record / "native-tool-results.json").write_text(json.dumps(responses, ensure_ascii=False, indent=2))
        self.passed("both installed native clients execute a read-only MCP tool concurrently through one authenticated upstream session")

        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-fixture-authorization"})
        with ThreadPoolExecutor(max_workers=2) as executor:
            a, b = executor.submit(codex_read), executor.submit(opencode_read)
            refreshed = {"codex": a.result(), "opencode": b.result()}
        assert all("fixture-workspace" in json.dumps(value, ensure_ascii=False) for value in refreshed.values())
        assert len(self.stats()["sessionVersions"]) == shared_sessions + 1
        assert self.cli("gateway", "status")["sessions"] == 1
        self.passed("a central credential update reconnects both installed clients through one fresh shared upstream session")

        self.cli("gateway", "pause")
        paused = codex_read()
        assert "暂停" in json.dumps(paused, ensure_ascii=False), paused
        self.cli("gateway", "resume")
        assert "fixture-workspace" in json.dumps(codex_read(), ensure_ascii=False)
        self.passed("native Codex tool calls respect shared pause and recover on resume without reconfiguration")
        return {"connection": connection, "thread": thread, "stats": self.stats()}

    def http(self, path, body=None):
        request = urllib.request.Request(f"http://127.0.0.1:{self.native_port}{path}",
            data=json.dumps(body).encode() if body is not None else None,
            headers={"Content-Type": "application/json", "Authorization": "Basic " +
                base64.b64encode(b"opencode:synthetic-native-acceptance").decode()})
        try:
            with urllib.request.urlopen(request, timeout=20) as response:
                return json.load(response)
        except urllib.error.HTTPError as error:
            (self.record / "opencode-http-error.json").write_bytes(error.read())
            raise

    def close_native(self):
        if getattr(self, "native_codex", None):
            self.native_codex.close()
        if getattr(self, "native_opencode", None):
            self.native_opencode.terminate()
            self.native_opencode.wait(timeout=10)
            self.native_log.close()
        if getattr(self, "model", None):
            self.model.shutdown()
            self.model.server_close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--codex", type=Path, required=True)
    parser.add_argument("--opencode", type=Path, required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = NativeSharedAgents(record)
    conclusion, error, ids = "PASS", None, {}
    try:
        ids = harness.run_native(args.codex, args.opencode)
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close_native()
        harness.close(False)
    native = [{"name": name, "binary_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
               "version": subprocess.check_output([str(path), "--version"], text=True).strip()}
              for name, path in [("codex", args.codex), ("opencode", args.opencode)]]
    report = {"conclusion": conclusion, "error": error, "executed_at": datetime.now(timezone.utc).isoformat(),
        "os": platform.platform(), "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "native_clients": native,
        "scenarios": harness.results, "commands": harness.commands, "ids": ids,
        "model_fixture": "deterministic loopback-only Chat Completions server; no external provider requests",
        "model_observations": harness.model.observations, "real_native_mcp_config_modified": False}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
