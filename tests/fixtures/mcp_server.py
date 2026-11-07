"""Synthetic MCP and OAuth service for mcplink end-to-end verification."""
import argparse
import base64
import hashlib
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import parse_qs, urlencode, urlparse


DOCUMENT = {"id": "weekly-fixture", "title": "mcplink 固定测试周报", "text": "本周完成 MCP 配置盘点、跨 Agent 接入和共享账号验证。", "workspace": "fixture-workspace"}
TOOLS = [
    {"name": "search_documents", "description": "Search synthetic documents", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}, "annotations": {"readOnlyHint": True}},
    {"name": "read_document", "description": "Read one synthetic document", "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "delay_ms": {"type": "integer"}}, "required": ["id"]}, "annotations": {"readOnlyHint": True}},
]


def handle(request):
    method = request.get("method")
    params = request.get("params") or {}
    if method == "initialize":
        return {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}, "resources": {}, "prompts": {}}, "serverInfo": {"name": "mcplink-fixture", "version": "1.0.0"}}
    if method == "server/discover":
        return {"protocolVersion": "2026-07-28", "capabilities": {"tools": {}, "resources": {}, "prompts": {}}, "serverInfo": {"name": "mcplink-fixture", "version": "1.0.0"}}
    if method == "ping":
        return {}
    if method == "tools/list":
        return {"tools": TOOLS}
    if method == "tools/call":
        args = params.get("arguments", {})
        if params.get("name") == "search_documents":
            output = {"account": "synthetic-work-account", "documents": [{"id": DOCUMENT["id"], "title": DOCUMENT["title"]}]}
        elif params.get("name") == "read_document" and args.get("id") == DOCUMENT["id"]:
            time.sleep(min(args.get("delay_ms", 0), 6000) / 1000)
            output = {**DOCUMENT, "account": "synthetic-work-account"}
        else:
            return {"content": [{"type": "text", "text": "Synthetic document not found"}], "isError": True}
        return {"content": [{"type": "text", "text": json.dumps(output, ensure_ascii=False)}], "isError": False}
    if method == "resources/list":
        return {"resources": [{"uri": "fixture://weekly", "name": "Weekly fixture", "mimeType": "text/plain"}]}
    if method == "resources/templates/list":
        return {"resourceTemplates": []}
    if method == "resources/read":
        return {"contents": [{"uri": "fixture://weekly", "mimeType": "text/plain", "text": DOCUMENT["text"]}]}
    if method == "prompts/list":
        return {"prompts": [{"name": "weekly_report", "description": "Synthetic weekly report"}]}
    if method == "prompts/get":
        return {"messages": [{"role": "user", "content": {"type": "text", "text": "Search and read the weekly fixture document."}}]}
    raise ValueError("Method not found")


class Handler(BaseHTTPRequestHandler):
    def log_message(self, *_):
        pass

    def send(self, status, body, headers=None):
        data = json.dumps(body, ensure_ascii=False).encode()
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(data)))
        for key, value in (headers or {}).items():
            self.send_header(key, value)
        self.end_headers()
        self.wfile.write(data)

    def do_GET(self):
        path = urlparse(self.path)
        origin = f"http://127.0.0.1:{self.server.server_port}"
        if path.path.startswith("/.well-known/oauth-protected-resource"):
            return self.send(200, {"resource": origin + "/mcp", "authorization_servers": [origin], "scopes_supported": ["documents:read"]})
        if path.path.startswith("/.well-known/oauth-authorization-server") or path.path.startswith("/.well-known/openid-configuration"):
            return self.send(200, {"issuer": origin, "authorization_endpoint": origin + "/authorize", "token_endpoint": origin + "/token", "registration_endpoint": origin + "/register", "response_types_supported": ["code"], "grant_types_supported": ["authorization_code", "refresh_token"], "token_endpoint_auth_methods_supported": ["none"], "code_challenge_methods_supported": ["S256"], "scopes_supported": ["documents:read", "offline_access"], "authorization_response_iss_parameter_supported": True})
        if path.path == "/authorize":
            query = {key: values[0] for key, values in parse_qs(path.query).items()}
            if query.get("code_challenge_method") != "S256":
                return self.send(400, {"error": "PKCE S256 required"})
            code = f"synthetic-code-{time.monotonic_ns()}"
            self.server.codes[code] = query
            redirect = query["redirect_uri"] + "?" + urlencode({"code": code, "state": query["state"], "iss": origin})
            self.send_response(302)
            self.send_header("Location", redirect)
            self.send_header("Content-Length", "0")
            self.end_headers()
            return
        if path.path == "/stats":
            return self.send(200, {"refreshes": self.server.refreshes, "calls": self.server.calls})
        return self.send(405, {"error": "POST required"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        path = urlparse(self.path).path
        origin = f"http://127.0.0.1:{self.server.server_port}"
        if path == "/register":
            body = json.loads(raw)
            return self.send(201, {"client_id": "mcplink-synthetic-client", "redirect_uris": body["redirect_uris"], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]})
        if path == "/token":
            body = {key: values[0] for key, values in parse_qs(raw.decode()).items()}
            if body.get("grant_type") == "authorization_code":
                original = self.server.codes.pop(body.get("code"), None)
                challenge = base64.urlsafe_b64encode(hashlib.sha256(body.get("code_verifier", "").encode()).digest()).decode().rstrip("=")
                if not original or challenge != original["code_challenge"] or body.get("redirect_uri") != original["redirect_uri"]:
                    return self.send(400, {"error": "invalid_grant"})
            elif body.get("grant_type") == "refresh_token":
                if body.get("refresh_token") != self.server.refresh_token:
                    return self.send(400, {"error": "invalid_grant"})
                self.server.refreshes += 1
            else:
                return self.send(400, {"error": "unsupported_grant_type"})
            self.server.access_token = f"synthetic-access-{time.monotonic_ns()}"
            self.server.refresh_token = f"synthetic-refresh-{time.monotonic_ns()}"
            return self.send(200, {"access_token": self.server.access_token, "refresh_token": self.server.refresh_token, "token_type": "Bearer", "expires_in": 3, "scope": "documents:read"})
        authorization = self.headers.get("Authorization", "")
        accepted = not self.server.auth or authorization == "synthetic-fixture-authorization" or authorization == "Bearer " + self.server.access_token
        if not accepted:
            challenge = None if path == "/mcp-no-challenge" else {"WWW-Authenticate": f'Bearer resource_metadata="{origin}/.well-known/oauth-protected-resource"'}
            return self.send(401, {"jsonrpc": "2.0", "id": None, "error": {"code": -32001, "message": "Synthetic authentication required"}}, challenge)
        try:
            request = json.loads(raw)
            if "id" not in request:
                return self.send(202, {})
            result = handle(request)
            if request.get("method") == "tools/call":
                self.server.calls += 1
            return self.send(200, {"jsonrpc": "2.0", "id": request["id"], "result": result})
        except ValueError:
            return self.send(200, {"jsonrpc": "2.0", "id": request.get("id"), "error": {"code": -32601, "message": "Method not found"}})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--stdio", action="store_true")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--auth", action="store_true")
    args = parser.parse_args()
    if args.stdio:
        for line in sys.stdin:
            request = json.loads(line)
            if "id" not in request:
                continue
            try:
                response = {"jsonrpc": "2.0", "id": request["id"], "result": handle(request)}
            except ValueError:
                response = {"jsonrpc": "2.0", "id": request["id"], "error": {"code": -32601, "message": "Method not found"}}
            print(json.dumps(response, ensure_ascii=False), flush=True)
        return
    server = ThreadingHTTPServer(("127.0.0.1", args.port), Handler)
    server.auth, server.codes, server.refreshes, server.calls = args.auth, {}, 0, 0
    server.access_token, server.refresh_token = "unissued-synthetic-access", "unissued-synthetic-refresh"
    print(json.dumps({"port": server.server_port}), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
