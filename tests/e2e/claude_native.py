"""Run native Claude MCP calls with a loopback, deterministic Messages provider.

The real CLI loads its generated native MCP config and dispatches tools. The
model responses are fixtures, so this is not a real-account model acceptance.
"""
import argparse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import subprocess
import sys
import threading
import traceback
import urllib.parse

from agent_cli import AgentCli
from run import FIXTURE


class Provider:
    def __init__(self):
        self.names = []
        self.results = []
        self.errors = []
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                value = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                if urllib.parse.urlsplit(self.path).path == "/v1/messages/count_tokens":
                    self.send_response(200)
                    self.send_header("Content-Type", "application/json")
                    self.end_headers()
                    self.wfile.write(b'{"input_tokens":1}')
                    return
                try:
                    assert urllib.parse.urlsplit(self.path).path == "/v1/messages"
                    names = sorted(t["name"] for t in value.get("tools", []) if t["name"].endswith("__read_document"))
                    assert len(names) == 2 and all(n.startswith("mcp__mcplink__") for n in names), names
                    fixture.names = names
                    results = [block for message in value["messages"] if isinstance(message.get("content"), list)
                               for block in message["content"] if block.get("type") == "tool_result"]
                    if not results:
                        content = [{"type": "tool_use", "id": f"fixture_call_{i}", "name": name,
                                    "input": {"id": "weekly-fixture"}} for i, name in enumerate(names)]
                        stop = "tool_use"
                    else:
                        assert len(results) == 2 and all(not b.get("is_error") and "weekly-fixture" in json.dumps(b) for b in results), results
                        fixture.results = results
                        content = [{"type": "text", "text": "NATIVE_MCP_FIXTURE_OK"}]
                        stop = "end_turn"
                    usage = {"input_tokens": 1, "output_tokens": 1}
                    message = {"id": "msg_fixture", "type": "message", "role": "assistant", "model": value["model"],
                               "content": content, "stop_reason": stop, "stop_sequence": None, "usage": usage}
                    self.send_response(200)
                    self.send_header("Content-Type", "text/event-stream" if value.get("stream") else "application/json")
                    self.end_headers()
                    if not value.get("stream"):
                        self.wfile.write(json.dumps(message).encode())
                        return

                    def event(kind, **data):
                        self.wfile.write((f"event: {kind}\ndata: " + json.dumps({"type": kind, **data}) + "\n\n").encode())
                    event("message_start", message=message | {"content": [], "stop_reason": None})
                    for index, block in enumerate(content):
                        if block["type"] == "tool_use":
                            event("content_block_start", index=index, content_block=block | {"input": {}})
                            event("content_block_delta", index=index, delta={"type": "input_json_delta", "partial_json": json.dumps(block["input"])})
                        else:
                            event("content_block_start", index=index, content_block={"type": "text", "text": ""})
                            event("content_block_delta", index=index, delta={"type": "text_delta", "text": block["text"]})
                        event("content_block_stop", index=index)
                    event("message_delta", delta={"stop_reason": stop, "stop_sequence": None}, usage={"output_tokens": 1})
                    event("message_stop")
                except Exception as error:
                    fixture.errors.append(str(error))
                    self.send_error(500)
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.server.serve_forever, daemon=True).start()
        self.url = f"http://127.0.0.1:{self.server.server_port}"

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--claude", type=Path, required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = AgentCli(record)
    provider = Provider()
    report = {"conclusion": "FAIL", "model_provider": "loopback fixture", "real_model_account_used": False}
    try:
        first = harness.rpc("connections.save", {"name": "claude_http", "definition": {
            "url": harness.url, "headers": {"Authorization": "synthetic-fixture-authorization"}}})["id"]
        second = harness.rpc("connections.save", {"name": "claude_stdio", "definition": {
            "command": sys.executable, "args": [str(FIXTURE), "--stdio"]}})["id"]
        entry = harness.rpc("gateway.save", {"connectionIds": [first, second]})
        harness.rpc("connections.probe", {"id": entry["id"]})
        _, applied = harness.changes(entry["id"], ["claude:user"], mode="shared")
        config = harness.home / ".claude/.claude.json"
        before = config.read_bytes()
        assert set(json.loads(before)["mcpServers"]) == {"mcplink"}
        assert b"synthetic-fixture-authorization" not in before
        project = record / "project"
        project.mkdir()
        environment = {k: v for k, v in harness.environment.items()
                       if not any(s in k.upper() for s in ["TOKEN", "API_KEY", "SECRET"])}
        environment.update(ANTHROPIC_API_KEY="synthetic-fixture-model-key", ANTHROPIC_BASE_URL=provider.url,
                           CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC="1", DISABLE_AUTOUPDATER="1", ENABLE_TOOL_SEARCH="false",
                           NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        allowed = ["mcp__mcplink__" + member["namespace"] + "__read_document" for member in entry["members"]]
        command = [str(args.claude.absolute()), "--bare", "-p", "Read the two fixture documents once.",
                   "--output-format", "json", "--max-turns", "4", "--model", "claude-sonnet-4-6",
                   "--strict-mcp-config", "--mcp-config", str(config), "--tools", "", "--allowedTools", ",".join(allowed)]
        result = subprocess.run(command, cwd=project, env=environment, capture_output=True, text=True, encoding="utf-8", timeout=90)
        (record / "native-output.log").write_text(result.stdout + result.stderr, encoding="utf-8")
        assert result.returncode == 0, "Native Claude run failed; see private native-output.log"
        assert not provider.errors, provider.errors
        assert set(provider.names) == set(allowed) and len(provider.results) == 2
        output = json.loads(result.stdout)
        assert not output.get("is_error") and output["result"] == "NATIVE_MCP_FIXTURE_OK", output
        # Claude may update its own preferences/launch metadata. Its MCP entry
        # must stay byte-for-byte equivalent as JSON; entry restoration tolerates
        # those unrelated native edits.
        assert json.loads(config.read_bytes())["mcpServers"] == json.loads(before)["mcpServers"]
        harness.passed("native Claude loads one generated MCP entry and actually calls both independent upstreams using a synthetic Messages provider")
        assert harness.rpc("changes.restore", {"operationId": applied["results"][0]["operationId"]})["status"] == "restored"
        assert not json.loads(config.read_text()).get("mcpServers")
        harness.passed("restoring the tested Claude native entry removes only its managed configuration")
        report.update(conclusion="PASS", scenarios=harness.results, native_tool_results=2)
    except Exception:
        traceback.print_exc()
    finally:
        provider.close()
        harness.close(False)
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(report["conclusion"], record, flush=True)
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
