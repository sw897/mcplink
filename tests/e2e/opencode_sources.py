"""Exercise current OpenCode JSONC sources through the CLI, Bridge and native client."""
import argparse
from datetime import datetime, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import platform
import re
import signal
import socket
import subprocess
import sys
import threading
import time
import traceback

from tool_calls import ToolCalls
from run import BINARY, FIXTURE


class OpenCodeSources(ToolCalls):
    def run(self):
        schema = self.cli("schema")
        sources = next(c for c in schema["command"]["commands"] if c["name"] == "sources")
        add = next(c for c in sources["commands"] if c["name"] == "add")
        assert "opencode" in next(a for a in add["arguments"] if a["name"] == "agent")["values"]
        assert all(a["name"] != "layout" for a in add["arguments"])
        self.cli("sources", "add", "--agent", "opencode", "--layout", "v2", exit_status=2)
        skill = self.cli("skill", "install", "--agent", "opencode")
        assert Path(skill["path"]) == self.home / ".config/opencode/skills/mcplink"
        assert self.cli("skill", "install", "--agent", "opencode")["status"] == "unchanged"
        assert not self.data.exists()
        self.passed("current OpenCode discovery and Skill installation are offline, without a historical version selector")

        config = self.home / ".config/opencode/opencode.jsonc"
        source = self.cli("sources", "add", "--agent", "opencode")
        assert not config.exists() and "layout" not in source
        row = next(s for s in self.cli("status")["sources"] if s["id"] == source["id"])
        assert not row["error"] and not row["exists"]
        self.passed("a missing OpenCode configuration registers without creating files or requiring a version")

        for text in [
            '{"$schema":"https://opencode.ai/config.json"}',
            '{"mcp":{}}',
            '{"mcp":{"servers":{}}}',
            '{"mcp":{"timeout":{"catalog":30000}}}',
        ]:
            config.write_text(text)
            row = next(s for s in self.cli("status")["sources"] if s["id"] == source["id"])
            assert not row["error"], row
            assert config.read_text() == text
        self.passed("serverless and timeout-only current configurations read without ambiguous-version warnings or edits")

        process = {"type": "local", "command": [sys.executable, str(FIXTURE), "--stdio"],
                   "environment": {"FIXTURE_PATH": "{env:MCPLINK_DATA_DIR}"},
                   "disabled": False, "codemode": False}
        text = ('// synthetic leading comment\n{\n  "$schema": "https://opencode.ai/config.json",\n'
                '  "unknown_setting": {"keep":42}, // preserve this setting\n'
                '  "mcp": {"servers": {\n    "native_fixture": ' + json.dumps(process)
                + ', // preserve other servers\n  }},\n}\n')
        config.write_text(text)
        imported = self.cli("connections", "import", "--source", source["id"], "--name", "native_fixture")["id"]
        definition = self.cli("connections", "show", imported)["definition"]
        assert definition["command"] == sys.executable and definition["args"] == [str(FIXTURE), "--stdio"]
        assert definition["env"]["FIXTURE_PATH"] == "$" + "{env:MCPLINK_DATA_DIR}"
        assert self.cli("connections", "probe", imported)["connection"]["status"] == "ready"
        self.passed("current JSONC imports command arrays and environment references through the real MCP SDK")

        connection = self.save("shared_notes")
        plan = self.cli("changes", "preview", "--connection", connection, "--target", source["id"])
        assert config.read_text() == text
        self.cli("changes", "apply", plan["planId"])
        actual = config.read_text()
        assert all(comment in actual for comment in ("// synthetic leading comment", "// preserve this setting", "// preserve other servers"))
        assert '"unknown_setting": {"keep":42}' in actual and '"disabled": false' in actual
        assert self.cli("connections", "import", "--source", source["id"], "--name", "shared_notes")["id"] == connection
        self.passed("preview/apply uses mcp.servers and preserves JSONC comments, unknown settings and unrelated servers")

        for action, expected in [("disable", False), ("enable", True)]:
            plan = self.cli("changes", "preview", "--connection", connection, "--target", source["id"], "--action", action)
            self.cli("changes", "apply", plan["planId"])
            status = self.cli("status")
            assert next(b for b in status["bindings"] if b["sourceId"] == source["id"] and b["name"] == "shared_notes")["enabled"] == expected
            assert '"enabled"' not in config.read_text() and "// preserve other servers" in config.read_text()
        self.passed("user configuration enable/disable uses disabled without emitting legacy fields")

        project = self.record / "current-project"
        project.mkdir()
        target = project / "opencode.jsonc"
        target.write_text('{\n// preserve project comment\n"mcp":{"timeout":{"catalog":30000},"servers":{}},\n'
                          '"username":"{env:UNCHANGED_FIXTURE}",\n"update":"disable",\n}\n')
        project_source = self.cli("sources", "add", "--agent", "opencode", "--scope", "project", "--project", str(project))
        assert "layout" not in project_source and Path(project_source["path"]) == target
        self.passed("explicit project sources use the current format without a layout parameter")

        self.cli("connections", "probe", connection)
        plan = self.cli("changes", "preview", "--connection", connection, "--target", source["id"],
                        "--target", project_source["id"], "--mode", "shared")
        results = self.cli("changes", "apply", plan["planId"])["results"]
        assert all(value["status"] == "applied" for value in results)
        assert '"disabled": false' in target.read_text() and '"type": "local"' in target.read_text()
        assert '"environment"' in target.read_text() and '"env"' not in target.read_text()
        assert '"command": [' in target.read_text() and '"args"' not in target.read_text()
        assert "synthetic-fixture-authorization" not in target.read_text() + config.read_text()
        assert '"username":"{env:UNCHANGED_FIXTURE}"' in target.read_text()
        self.passed("user and project shared entries use current command/environment fields and credential references")

        for action, expected in [("disable", False), ("enable", True)]:
            plan = self.cli("changes", "preview", "--connection", connection, "--target", project_source["id"], "--action", action)
            self.cli("changes", "apply", plan["planId"])
            status = self.cli("status")
            assert next(b for b in status["bindings"] if b["sourceId"] == project_source["id"] and b["name"] == "shared_notes")["enabled"] == expected
            assert '"enabled"' not in target.read_text() and "// preserve project comment" in target.read_text()
        self.passed("project enable/disable preserves comments and uses inverse disabled semantics")

        clients = [self.bridge(connection, "opencode-shared-" + str(i)) for i in range(2)]
        for client in clients:
            assert "result" in client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
        before = {path: path.read_bytes() for path in (config, target)}
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-fixture-authorization"})
        runtime = json.loads((self.data / "runtime.json").read_text())
        os.kill(runtime["pid"], signal.SIGTERM)
        time.sleep(.4)
        assert not self.invoke(connection, "read_document", {"id": "weekly-fixture"})["output"]["isError"]
        for client in clients:
            assert "result" in client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
        assert all(path.read_bytes() == value for path, value in before.items())
        self.passed("credential updates and backend restart preserve established Bridges and native references")

        plan = self.cli("changes", "preview", "--connection", connection, "--target", project_source["id"], "--action", "remove")
        removal = self.cli("changes", "apply", plan["planId"])["results"][0]
        assert "shared_notes" not in target.read_text()
        self.cli("changes", "restore", removal["operationId"])
        assert "shared_notes" in target.read_text() and "// preserve project comment" in target.read_text()
        self.passed("entry removal and restore retain surrounding JSONC")

        planned = self.cli("changes", "preview", "--connection", connection, "--target", project_source["id"])
        target.write_text(target.read_text() + "// concurrent edit\n")
        self.cli("changes", "apply", planned["planId"], exit_status=1)
        assert target.read_text().endswith("// concurrent edit\n")
        self.passed("current JSONC writes retain concurrent-file protection")

        invalid = self.record / "invalid"
        invalid.mkdir()
        invalid_file = invalid / "opencode.jsonc"
        for invalid_text in [
            '{"mcp":{"legacy":{"type":"local","command":["fixture"]}}}',
            '{"mcp":{"servers":{},"legacy":{"type":"local","command":["fixture"]}}}',
            '{"mcp":{"servers":{"type":"local","command":["fixture"]}}}',
            '{"mcp":{"servers":{"enabled":true}}}',
            '{"mcp":{"timeout":{"type":"local","command":["fixture"]}}}',
            '{"mcp":{"timeout":{"catalog":0}}}',
            '{"mcp":{"timeout":{"catalog":9007199254740992}}}',
            '{"mcp":{"servers":[]}}',
            '{"mcp":{},"mcp":{}}',
            "{not_json: true}",
        ]:
            invalid_file.write_text(invalid_text)
            self.cli("sources", "add", "--agent", "opencode", "--path", str(invalid_file), exit_status=1)
            assert invalid_file.read_text() == invalid_text
        invalid_file.write_text('{"mcp":{"servers":{}}}')
        self.cli("rpc", "sources.add", "--params",
                 json.dumps({"agent": "opencode", "path": str(invalid_file), "layout": "v1"}), exit_status=1)
        self.passed("legacy/mixed tables, historical RPC selectors and malformed JSON fail without rewriting files")

        unsupported = {
            "legacy_enabled": {"type": "remote", "url": "https://example.invalid/mcp", "enabled": True},
            "legacy_oauth": {"type": "remote", "url": "https://example.invalid/mcp", "oauth": {"clientId": "synthetic-client"}},
            "unknown_policy": {"type": "remote", "url": "https://example.invalid/mcp", "future_native_policy": True},
            "native_codemode": {"type": "remote", "url": "https://example.invalid/mcp", "codemode": True},
            "literal_variable": {"type": "remote", "url": "https://example.invalid/" + "$" + "{MCPLINK_DATA_DIR}"},
            "file_macro": {"type": "remote", "url": "https://example.invalid/mcp", "headers": {"Authorization": "{file:synthetic-secret}"}},
            "modern_protocol": {"type": "remote", "url": "https://example.invalid/mcp", "protocol": "2026-07-28"},
            "native_oauth": {"type": "remote", "url": "https://example.invalid/mcp", "oauth": {"client_secret": "synthetic-special-secret"}},
            "native_timeout": {"type": "remote", "url": "https://example.invalid/mcp", "timeout": {"catalog": 3456}},
        }
        invalid_file.write_text(json.dumps({"mcp": {"servers": unsupported}}))
        other = self.cli("sources", "add", "--agent", "opencode", "--path", str(invalid_file))
        for name in unsupported:
            error = self.cli("connections", "import", "--source", other["id"], "--name", name, exit_status=1)
            assert "synthetic-special-secret" not in json.dumps(error)
        invalid_file.write_text(json.dumps({"mcp": {"timeout": {"catalog": 3456}, "servers": {
            "uses_defaults": {"type": "local", "command": [sys.executable, str(FIXTURE), "--stdio"]}}}}))
        self.cli("connections", "import", "--source", other["id"], "--name", "uses_defaults", exit_status=1)
        self.passed("legacy fields and non-equivalent macros, OAuth, protocol and timeouts fail without secret echo")

        self.check_old_backend()
        self.check_native(target.read_text())
        return {"connection": connection, "user_source": source["id"], "project_source": project_source["id"]}

    def check_old_backend(self):
        runtime_file = self.data / "runtime.json"
        original = runtime_file.read_bytes()
        runtime = json.loads(original)
        requests = []

        class OldBackend(BaseHTTPRequestHandler):
            def do_GET(self):
                body = json.dumps({"application": "mcplink", "instance": runtime["instance"], "managementApi": 5}).encode()
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                requests.append(self.path)
                self.send_response(200)
                self.send_header("Content-Type", "application/json")
                self.end_headers()
                self.wfile.write(b'{"ok":{}}')

            def log_message(self, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), OldBackend)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            runtime_file.write_text(json.dumps({**runtime, "port": server.server_port}))
            result = self.cli("sources", "add", "--agent", "opencode", exit_status=1)
            assert "旧后台" in result["error"]["message"] and not requests
        finally:
            runtime_file.write_bytes(original)
            server.shutdown()
            server.server_close()
            thread.join(timeout=5)
        self.passed("API 5 backends reject current OpenCode operations before an unsupported RPC is sent")

    def check_native(self, config_text):
        if not (executable := os.environ.get("MCPLINK_E2E_OPENCODE")):
            return
        native_root = self.record / "native"
        native_home = native_root / "home"
        native_config = native_home / ".config/opencode"
        native_config.mkdir(parents=True)
        native_project = native_root / "project"
        native_project.mkdir()
        subprocess.run(["git", "init", "--quiet", str(native_project)], check=True, capture_output=True)
        (native_config / "opencode.jsonc").write_text(config_text)
        environment = self.environment.copy()
        for name in ("OPENCODE_CONFIG", "OPENCODE_CONFIG_CONTENT", "OPENCODE_SERVER",
                     "OPENCODE_SERVER_USERNAME", "OPENCODE_SERVER_PASSWORD"):
            environment.pop(name, None)
        environment.update({
            "OPENCODE_CONFIG_DIR": str(native_config), "OPENCODE_TEST_HOME": str(native_home),
            "XDG_CONFIG_HOME": str(native_home / ".config"), "XDG_DATA_HOME": str(native_home / ".local/share"),
            "XDG_CACHE_HOME": str(native_home / ".cache"), "XDG_STATE_HOME": str(native_home / ".local/state"),
            "OPENCODE_PASSWORD": "synthetic-opencode-e2e", "UNCHANGED_FIXTURE": "synthetic-user"})
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        with (self.record / "native-server.log").open("w") as log:
            server = subprocess.Popen([executable, "serve", "--hostname", "127.0.0.1", "--port", str(port)],
                                      cwd=native_project, env=environment, stdout=log, stderr=subprocess.STDOUT)
            try:
                deadline = time.monotonic() + 50
                while True:
                    result = subprocess.run([executable, "--server", f"http://127.0.0.1:{port}", "mcp", "list"],
                                            cwd=native_project, env=environment, capture_output=True, text=True, timeout=30)
                    output = re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", result.stdout)
                    if result.returncode == 0 and any("shared_notes" in line and re.search(r"\bconnected\b", line, re.I)
                                                      for line in output.splitlines()):
                        break
                    assert time.monotonic() < deadline, result.stdout + result.stderr
                    time.sleep(.25)
            finally:
                server.terminate()
                try:
                    server.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    server.kill()
                    server.wait(timeout=5)
        (self.record / "native-load.txt").write_text(result.stdout + result.stderr)
        native_version = subprocess.run([executable, "--version"], env=environment, capture_output=True,
                                        text=True, timeout=15).stdout.strip()
        self.passed("the current native OpenCode client loads the generated configuration and connects through the Bridge",
                    {"version": native_version, "binary_sha256": hashlib.sha256(Path(executable).read_bytes()).hexdigest(),
                     "scope": "native load and MCP connection, no model/tool execution claim"})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = OpenCodeSources(record)
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
              "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "scenarios": harness.results,
              "commands": harness.commands, "ids": ids, "native_client_requested": bool(os.environ.get("MCPLINK_E2E_OPENCODE")),
              "real_native_mcp_config_modified": False}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
