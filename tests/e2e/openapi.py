"""Offline import, credential-safe HTTP execution and protected transport migration."""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import sys
import traceback

from http_api import HttpApi
from run import FIXTURE


class OpenApi(HttpApi):
    def run(self):
        offline = self.record / "offline-import-data"
        document = {"openapi": "3.1.0", "info": {"title": "Fixture import"},
                    "servers": [{"url": self.endpoint + "/v1"}],
                    "components": {"securitySchemes": {"key": {"type": "http", "scheme": "bearer"}}},
                    "security": [{"key": []}], "paths": {
            "/notes/{id}": {"get": {"operationId": "read_note", "parameters": [
                {"name": "id", "in": "path", "required": True, "schema": {"type": "string"}},
                {"name": "tags", "in": "query", "schema": {"type": "array", "items": {"type": "string"}}}],
                "responses": {"200": {"content": {"application/json": {"schema": {"type": "object"}}}}}}},
            "/optional": {"post": {"operationId": "optional_body", "requestBody": {"content": {
                "application/problem+json": {"schema": {"type": "object", "properties": {"count": {"type": "integer", "default": 8}}}}}},
                "responses": {"200": {"content": {"application/json": {}}}}}},
            "/upload": {"post": {"requestBody": {"content": {"multipart/form-data": {"schema": {"type": "object"}}}}, "responses": {"200": {}}}}
        }}
        source = json.dumps(document)
        preview = self.cli("--data-dir", str(offline), "http-api", "import", "--stdin", value={"source": source})
        rpc_preview = self.cli("rpc", "--stdin", value={"method": "httpApi.import", "params": {"source": source}})
        assert rpc_preview == preview
        assert preview["definition"] is None and not any(o["selected"] for o in preview["operations"])
        assert preview["networkRequests"] == preview["configurationWrites"] == 0
        assert not offline.exists() and not self.requests
        assert next(o for o in preview["operations"] if o["key"] == "POST /upload")["issue"]
        self.cli("http-api", "import", "--stdin", value={"source": "private-invalid-content: ["}, exit_status=2)
        selected = self.cli("http-api", "import", "--stdin", value={"source": source, "operations": ["GET /notes/{id}", "POST /optional"],
                             "toolNames": {"GET /notes/{id}": "read_renamed"}})
        assert selected["credentialInputs"][0]["header"] == "Authorization"
        definition = selected["definition"]
        assert definition["headers"] == {} and definition["includeTools"] == ["read_renamed", "optional_body"]
        assert "default" not in json.dumps(definition["httpApi"])
        definition["headers"] = {"Authorization": "Bearer fixture-http-api"}
        connection = self.cli("connections", "save", "--stdin", value={"name": "imported_api", "definition": definition})["id"]
        assert not self.requests
        self.cli("connections", "tools", connection)
        assert not self.requests
        self.cli("call", connection, "read_renamed", "--stdin", value={"path": {"id": "a/b"}, "query": {"tags": ["x", "y"]}})
        assert self.requests[-1]["path"] == "/v1/notes/a%2Fb" and self.requests[-1]["query"] == {"tags": ["x", "y"]}
        self.cli("call", connection, "optional_body", "--stdin", value={})
        assert self.requests[-1]["body"] is None
        self.cli("call", connection, "optional_body", "--stdin", value={"body": {"count": 3}})
        assert self.requests[-1]["body"] == {"count": 3}
        assert self.requests[-1]["contentType"] == "application/problem+json"
        self.passed("offline import defaults to no selection; explicit selection preserves base paths, repeated arrays and optional JSON bodies")

        swagger = self.record / "swagger.yaml"
        swagger.write_text("swagger: '2.0'\ninfo: {title: Fixture, version: '1'}\nhost: '127.0.0.1:" + str(self.api.http.server_port) +
                           "'\nschemes: [http]\nbasePath: /v2\npaths:\n  /notes:\n    get:\n      operationId: list_notes\n      produces: [application/json]\n      responses: {'200': {description: ok}}\n")
        imported = self.cli("http-api", "import", "--file", str(swagger), "--operation", "GET /notes")
        assert imported["definition"]["httpApi"]["tools"][0]["path"] == "/v2/notes"
        unsupported = self.cli("http-api", "import", "--stdin", value={"source": source, "operations": ["POST /upload"]})
        assert unsupported["definition"] is None and unsupported["issues"]
        collision_doc = json.loads(source)
        collision_doc["paths"]["/optional"]["post"]["operationId"] = "read_note"
        collision = self.cli("http-api", "import", "--stdin", value={"source": json.dumps(collision_doc), "all": True})
        assert collision["definition"] is None and any(i["code"] == "tool_name_collision" for i in collision["issues"])
        self.passed("Swagger YAML file import works; unsupported selections and tool name collisions produce no draft")

        tools = [{"name": outcome.replace("-", "_"), "description": "Generic response policy fixture", "inputSchema": {"type": "object", "additionalProperties": False},
                  "method": "GET", "path": "/condition/" + outcome,
                  "response": {"type": "json", "success": {"pointer": "/code", "equals": 0}, "sensitivePaths": ["/data/private"]}}
                 for outcome in ["ok", "failed", "missing", "wrong-type"]]
        tools.append({"name": "text", "description": "Text reflection", "inputSchema": {"type": "object"}, "method": "GET", "path": "/condition/text", "response": {"type": "text"}})
        secure = self.save_api("response_policy", tools)["id"]
        result = self.cli("call", secure, "ok", "--stdin", value={})["output"]
        assert result["structuredContent"]["data"]["private"] == "[redacted]"
        assert "fixture-http-api" not in json.dumps(result) and "fixture-business-private" not in json.dumps(result)
        for operation in ["failed", "missing", "wrong_type"]:
            result = self.cli("call", secure, operation, "--stdin", value={}, exit_status=1)
            assert "fixture-http-api" not in json.dumps(result) and "fixture-business-private" not in json.dumps(result)
            detail = self.cli("connections", "show", secure)
            assert detail["connection"]["status"] == "error"
            assert detail["connection"]["lastError"] == "上游响应未满足此服务配置的成功条件"
        reflected = self.cli("call", secure, "text", "--stdin", value={})["output"]
        assert "fixture-http-api" not in json.dumps(reflected) and "[redacted]" in json.dumps(reflected)
        direct = self.bridge(secure, "response-policy")
        result = direct.call("tools/call", {"name": "failed", "arguments": {}})["result"]
        assert result["isError"] and "fixture-http-api" not in json.dumps(result)
        entry = self.cli("gateway", "save", "--stdin", value={"connectionIds": [secure]})
        shared = self.bridge(entry["id"], "response-policy-shared")
        name = entry["members"][0]["namespace"] + "__ok"
        result = shared.call("tools/call", {"name": name, "arguments": {}})["result"]
        assert not result["isError"] and "fixture-http-api" not in json.dumps(result)
        self.passed("configured scalar success conditions reject missing/mismatched fields; credential and sensitive-path filters protect CLI, MCP and persisted error status")

        original = self.cli("connections", "save", "--stdin", value={"name": "transport_migration", "definition": {
            "command": sys.executable, "args": [str(FIXTURE), "--stdio"], "env": {"FIXTURE_KEY": "Bearer fixture-http-api"}}})["id"]
        old = self.cli("connections", "show", original)
        mappings = [{"from": {"field": "env", "name": "FIXTURE_KEY"}, "to": {"field": "headers", "name": "Authorization"}}]
        migrated = {"url": self.endpoint, "httpApi": {"tools": tools}, "headers": {}, "editVersion": old["definition"]["editVersion"]}
        before = self.cli("connections", "show", original)
        for bad in [mappings + mappings, [{"from": {"field": "env", "name": "MISSING"}, "to": {"field": "headers", "name": "Authorization"}}]]:
            self.cli("connections", "save", "--stdin", value={"id": original, "name": "transport_migration", "definition": migrated, "credentialMappings": bad}, exit_status=1)
            assert self.cli("connections", "show", original) == before
        self.cli("connections", "save", "--stdin", value={"name": "new_migration", "definition": migrated, "credentialMappings": mappings}, exit_status=1)
        saved = self.cli("connections", "save", "--stdin", value={"id": original, "name": "transport_migration", "definition": migrated, "credentialMappings": mappings})
        assert saved["id"] == original and saved["transport"] == "http-api"
        masked = self.cli("connections", "show", original)
        assert masked["definition"]["env"] == {} and masked["definition"]["headers"]["Authorization"] == "••••••••"
        self.cli("call", original, "ok", "--stdin", value={})
        self.cli("connections", "save", "--stdin", value={"id": original, "name": "transport_migration", "definition": migrated, "credentialMappings": mappings}, exit_status=1)
        changed = masked["definition"] | {"url": "https://other.invalid"}
        self.cli("connections", "save", "--stdin", value={"id": original, "name": "transport_migration", "definition": changed,
                 "credentialMappings": [{"from": {"field": "headers", "name": "Authorization"}, "to": {"field": "env", "name": "OTHER"}}]}, exit_status=1)
        assert self.cli("connections", "show", original)["definition"]["url"] == self.endpoint
        self.passed("same-ID credential migration stays in protected storage; duplicate, missing, stale, new-ID and changed-origin requests leave the original intact")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    args.record.mkdir(parents=True)
    harness = OpenApi(args.record.resolve())
    error = None
    try:
        harness.run()
    except Exception:
        error = traceback.format_exc()
    finally:
        harness.close_http()
        harness.close(False)
    report = {"conclusion": "FAIL" if error else "PASS", "executedAt": datetime.now(timezone.utc).isoformat(),
              "results": harness.results, "error": error, "requests": len(harness.requests)}
    (args.record / "report.json").write_text(json.dumps(report, indent=2), encoding="utf-8")
    print(json.dumps({"conclusion": report["conclusion"], "report": str(args.record / "report.json"), "error": error}))
    raise SystemExit(1 if error else 0)


if __name__ == "__main__":
    main()
