"""Exercise declarative HTTP APIs through the public CLI, aggregate entry and MCP Bridge."""
import argparse
import base64
from datetime import datetime, timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
from pathlib import Path
import threading
import time
import traceback
import urllib.parse

from agent_cli import AgentCli


PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+jRZkAAAAASUVORK5CYII=")
SCHEMA = {"type": "object", "properties": {"id": {"type": "string"}, "count": {"type": "integer", "minimum": 1}, "enabled": {"type": "boolean"}, "items": {"type": "array", "items": {"type": "integer"}}}, "required": ["id", "count", "enabled", "items"], "additionalProperties": False}


def tool(name="echo", path="/echo/{id}", response=None):
    return {"name": name, "description": "Exercise typed HTTP API mapping", "inputSchema": SCHEMA,
            "method": "POST", "path": path, "pathParams": {"id": {"$input": "/id"}},
            "query": {"count": {"$input": "/count"}, "optional": {"$input": "/optional"}},
            "body": {"enabled": {"$input": "/enabled"}, "items": {"$input": "/items"}},
            "response": response or {"type": "json"}}


class ApiServer:
    def __init__(self):
        self.requests = []
        harness = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *args):
                pass

            def do_POST(self):
                self.respond()

            def do_GET(self):
                self.respond()

            def respond(self):
                raw = self.rfile.read(int(self.headers.get("Content-Length", 0)))
                path = urllib.parse.urlsplit(self.path)
                harness.requests.append({"path": path.path, "query": urllib.parse.parse_qs(path.query), "body": json.loads(raw) if raw else None, "authorization": self.headers.get("Authorization")})
                if path.path.startswith("/slow/"):
                    time.sleep(1)
                status, mime, body = 200, "application/json", json.dumps(harness.requests[-1]).encode()
                if self.headers.get("Authorization") != "Bearer fixture-http-api":
                    status, body = 401, b'{"secret":"upstream-private-body"}'
                elif path.path == "/png":
                    mime, body = "image/png", PNG
                elif path.path == "/base64":
                    body = json.dumps({"data": [{"b64_json": base64.b64encode(PNG).decode()}]}).encode()
                elif path.path == "/invalid-json":
                    body = b"upstream-private-body"
                elif path.path == "/failure":
                    status, body = 503, b"upstream-private-body"
                elif path.path == "/redirect":
                    status, body = 302, b""
                elif path.path == "/large":
                    body = b"x" * (8 * 1024 * 1024 + 1)
                self.send_response(status)
                self.send_header("Content-Type", mime)
                if status == 302:
                    self.send_header("Location", "/must-not-follow")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                try:
                    self.wfile.write(body)
                except (BrokenPipeError, ConnectionResetError):
                    pass

        self.http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.http.serve_forever, daemon=True).start()
        self.endpoint = f"http://127.0.0.1:{self.http.server_port}"

    def close(self):
        self.http.shutdown()
        self.http.server_close()


class HttpApi(AgentCli):
    def __init__(self, record):
        super().__init__(record)
        self.api = ApiServer()
        self.requests = self.api.requests
        self.endpoint = self.api.endpoint

    def save_api(self, name, tools, **options):
        return self.cli("connections", "save", "--stdin", value={"name": name, "definition": {
            "url": self.endpoint, "headers": {"Authorization": "Bearer fixture-http-api"},
            "httpApi": {"tools": tools, **options}}})

    def run(self):
        payload = {"id": "a/b ?", "count": 2, "enabled": False, "items": [1, 2]}
        tools = [tool(), {"name": "image", "description": "Get a PNG", "inputSchema": {"type": "object", "additionalProperties": False}, "method": "GET", "path": "/png", "response": {"type": "image"}}]
        for invalid in [[tool(), tool()], [tool() | {"path": "//example.com/escape"}], [tool() | {"inputSchema": {"type": "object", "$ref": "https://example.com/schema"}}], [tool() | {"pathParams": {}}], [tool() | {"annotations": {"readOnlyHint": "invalid"}}]]:
            self.cli("connections", "save", "--stdin", value={"name": "invalid_api", "definition": {"url": self.endpoint, "httpApi": {"tools": invalid}}}, exit_status=1)
        assert not self.requests
        self.passed("invalid declarations, external schemas and path escapes fail before dispatch")
        saved = self.save_api("http_fixture", tools)
        connection = saved["id"]
        assert saved["transport"] == "http-api"
        before = len(self.requests)
        listed = self.cli("connections", "tools", connection)
        assert len(listed["tools"]) == 2 and len(self.requests) == before
        assert listed["verification"]["upstream"] == "not_checked"
        assert listed["verification"]["probe"] is None
        self.cli("connections", "probe", connection, exit_status=1)
        self.cli("credentials", "login", connection, exit_status=1)
        assert len(self.requests) == before
        self.passed("HTTP tool discovery is local and never claims network/authentication verification")

        probed = self.save_api("confirmed_probe", [tools[0]], probe={"tool": "echo", "arguments": payload})["id"]
        before = len(self.requests)
        preview = self.rpc("connections.tools", {"id": probed})
        assert len(self.requests) == before
        assert preview["verification"]["probe"] == {"tool": "echo", "method": "POST", "path": "/echo/{id}"}
        assert "fixture-http-api" not in json.dumps(preview)
        revision = preview["connection"]["revision"]
        self.cli("credentials", "update", probed, "--stdin", value={"value": "Bearer fixture-http-api"})
        error = self.rpc("connections.probe", {"id": probed, "expectedRevision": revision}, error=True)
        assert "变化" in error and len(self.requests) == before
        preview = self.rpc("connections.tools", {"id": probed})
        verified = self.rpc("connections.probe", {"id": probed, "expectedRevision": preview["connection"]["revision"]})
        assert len(self.requests) == before + 1
        assert verified["verification"]["upstream"] == "probe_call" and verified["connection"]["status"] == "ready"
        self.passed("probe preparation exposes only the operation identity and stale confirmation fails before HTTP dispatch")

        result = self.cli("call", connection, "echo", "--stdin", value=payload)
        actual = json.loads(result["output"]["content"][0]["text"])
        assert actual["path"] == "/echo/a%2Fb%20%3F"
        assert actual["query"] == {"count": ["2"]}
        assert actual["body"] == {"enabled": False, "items": [1, 2]}
        assert actual["authorization"] == "Bearer fixture-http-api"
        assert self.cli("connections", "probe", connection)["verification"]["upstream"] == "previous_call"
        self.passed("path/query/JSON mapping preserves encoding and types; actual execution verifies upstream")

        bridge = self.bridge(connection, "http-api-direct")
        before = len(self.requests)
        bad = bridge.call("tools/call", {"name": "echo", "arguments": payload | {"count": 0}})
        assert "error" in bad and len(self.requests) == before
        self.cli("call", connection, "echo", "--stdin", value=payload | {"count": 0}, exit_status=2)
        assert len(self.requests) == before
        image = bridge.call("tools/call", {"name": "image", "arguments": {}})["result"]
        assert image["content"][0]["type"] == "image"
        assert base64.b64decode(image["content"][0]["data"]) == PNG
        self.passed("native MCP validates before HTTP dispatch and returns typed image content")

        other = self.save_api("second_api", [tools[1]])["id"]
        entry = self.cli("gateway", "save", "--stdin", value={"connectionIds": [connection, other]})
        namespaces = {m["connectionId"]: m["namespace"] for m in entry["members"]}
        client = self.bridge(entry["id"], "http-api-aggregate")
        names = {row["name"] for row in client.call("tools/list")["result"]["tools"]}
        assert len(names) == 3
        image_name = namespaces[connection] + "__image"
        assert client.call("tools/call", {"name": image_name, "arguments": {}})["result"]["content"][0]["type"] == "image"
        output = self.record / "generated"
        generated = self.cli("cli", "generate", entry["id"], "--name", "http-tools", "--output", str(output), "--tool", image_name)
        manifest = output / "http-tools.mcplink.json"
        assert manifest.exists(), generated
        self.cli("cli", "check", str(manifest))
        self.passed("aggregate MCP entry and generated CLI reuse the member connection and credentials")

        self.cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--mode", "direct", exit_status=1)
        for agent in ["qwenwork", "doubaowork"]:
            self.cli("connections", "export-config", "--agent", agent, "--mode", "direct", "--connection", connection, exit_status=1)
        plan = self.cli("changes", "preview", "--connection", entry["id"], "--target", "codex:user", "--target", "hermes:user", "--mode", "shared")
        self.cli("changes", "apply", plan["planId"])
        self.passed("both native configs receive one managed entry; raw HTTP APIs cannot be exported as native MCP")

        for path, response in [("/failure", {"type": "json"}), ("/invalid-json", {"type": "json"}), ("/redirect", {"type": "json"}), ("/large", {"type": "json"}), ("/invalid-json", {"type": "image"})]:
            definition = tool("failure", path, response)
            definition["pathParams"] = {}
            failed = self.save_api("failure_api", [definition])["id"]
            before = len(self.requests)
            result = self.cli("call", failed, "failure", "--stdin", value=payload, exit_status=1)
            assert len(self.requests) == before + 1
            assert "upstream-private-body" not in json.dumps(result)
        assert all(r["path"] != "/must-not-follow" for r in self.requests)
        self.passed("HTTP/JSON/type/size failures are bounded, redacted, and never redirected or retried")

        encoded = {"name": "encoded", "description": "Read base64 image in JSON", "inputSchema": {"type": "object"}, "method": "GET", "path": "/base64", "response": {"type": "base64Image", "pointer": "/data/0/b64_json", "mimeType": "image/png"}}
        encoded_id = self.save_api("encoded_image", [encoded])["id"]
        result = self.cli("call", encoded_id, "encoded", "--stdin", value={})
        assert base64.b64decode(result["output"]["content"][0]["data"]) == PNG

        detail = self.cli("connections", "show", connection)
        self.cli("connections", "save", "--stdin", value={"id": connection, "name": "http_fixture", "definition": detail["definition"]})
        assert self.cli("call", connection, "echo", "--stdin", value=payload)
        self.cli("credentials", "disconnect", connection)
        self.cli("call", connection, "echo", "--stdin", value=payload, exit_status=1)
        self.cli("credentials", "update", connection, "--stdin", value={"value": "Bearer fixture-http-api"})
        assert self.cli("call", connection, "echo", "--stdin", value=payload)
        self.cli("gateway", "pause")
        self.cli("call", connection, "echo", "--stdin", value=payload, exit_status=1)
        self.cli("gateway", "resume")
        self.passed("base64 mapping, masked edit, credential recovery and shared pause use the existing lifecycle")

    def close_http(self):
        self.api.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    args.record.mkdir(parents=True)
    harness = HttpApi(args.record.resolve())
    error = None
    try:
        harness.run()
    except Exception:
        error = traceback.format_exc()
    finally:
        harness.close_http()
        harness.close(False)
    report = {"conclusion": "FAIL" if error else "PASS", "executedAt": datetime.now(timezone.utc).isoformat(), "scenarios": harness.results, "error": error, "requests": len(harness.requests)}
    (args.record / "report.json").write_text(json.dumps(report, indent=2))
    print(json.dumps({"conclusion": report["conclusion"], "report": str(args.record / "report.json"), "error": error}))
    raise SystemExit(1 if error else 0)


if __name__ == "__main__":
    main()
