"""Synthetic MCP and OAuth service for mcplink end-to-end verification."""
import argparse
import base64
import faulthandler
import hashlib
import json
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from socketserver import TCPServer
from urllib.parse import parse_qs, urlencode, urlparse


DOCUMENT = {"id": "weekly-fixture", "title": "mcplink 固定测试周报", "text": "本周完成 MCP 配置盘点、跨 Agent 接入和共享账号验证。", "workspace": "fixture-workspace"}
TOOLS = [
    {"name": "search_documents", "description": "Search synthetic documents", "inputSchema": {"type": "object", "properties": {"query": {"type": "string"}}, "required": ["query"]}, "annotations": {"readOnlyHint": True}},
    {"name": "read_document", "description": "Read one synthetic document", "inputSchema": {"type": "object", "properties": {"id": {"type": "string"}, "delay_ms": {"type": "integer"}}, "required": ["id"]}, "annotations": {"readOnlyHint": True}},
]

CALL_TOOLS = [
    {"name":"inspect_payload","description":"Echo a typed synthetic payload","inputSchema":{
        "type":"object","$defs":{"item":{"type":"object","properties":{"label":{"type":"string","minLength":2},"weight":{"type":"number","minimum":0}},"required":["label","weight"],"additionalProperties":False}},
        "properties":{"items":{"type":"array","items":{"$ref":"#/$defs/item"},"minItems":1},"enabled":{"type":"boolean"},"count":{"type":"integer","minimum":1,"maximum":4},"choice":{"enum":["a","b"]},"optional":{"anyOf":[{"type":"null"},{"type":"string"}]}},
        "required":["items","enabled","count","choice"],"additionalProperties":False}},
    {"name":"invalid_schema","inputSchema":{"type":"object","properties":{"value":{"type":"not-a-type"}}}},
    {"name":"external_schema","inputSchema":{"type":"object","properties":{"value":{"$ref":"http://127.0.0.1:9/private-schema"}}}},
]

GENERATED_TOOLS = [
    {"name":"odd_parameters","inputSchema":{"type":"object","properties":{"stdin":{"type":"string"},"odd/name":{"type":"boolean"}},"required":["stdin","odd/name"]}},
    {"name":"schema_secrets","description":"synthetic-schema-description-private","inputSchema":{"type":"object","properties":{"value":{"type":"string","default":"synthetic-schema-default-private","examples":["synthetic-schema-example-private"],"enum":["synthetic-schema-enum-private"]}}}},
]


class LoopbackHTTPServer(ThreadingHTTPServer):
    def server_bind(self):
        # The synthetic service only uses its bound loopback address; do not
        # make startup depend on the host's reverse DNS configuration.
        TCPServer.server_bind(self)
        self.server_name, self.server_port = self.server_address[:2]


def handle(request, endpoint="", origin=""):
    method = request.get("method")
    params = request.get("params") or {}
    if method == "initialize":
        return {"protocolVersion": "2025-06-18", "capabilities": {"tools": {}, "resources": {}, "prompts": {}}, "serverInfo": {"name": "mcplink-fixture", "version": "1.0.0"}}
    if method == "server/discover":
        return {"protocolVersion": "2026-07-28", "capabilities": {"tools": {}, "resources": {}, "prompts": {}}, "serverInfo": {"name": "mcplink-fixture", "version": "1.0.0"}}
    if method == "ping":
        return {}
    if method == "tools/list":
        if endpoint in {"/mcp-tools", "/mcp-slow", "/mcp-cycle", "/mcp-duplicates", "/mcp-generated", "/mcp-generated-changed"}:
            if endpoint == "/mcp-cycle":
                return {"tools":[],"nextCursor":"repeat"}
            if endpoint == "/mcp-duplicates":
                return {"tools":[TOOLS[0],TOOLS[0]]}
            tools = TOOLS + CALL_TOOLS
            if endpoint.startswith("/mcp-generated"):
                tools = tools + GENERATED_TOOLS
            if endpoint == "/mcp-generated-changed":
                tools = [{**tool,"inputSchema":{**tool["inputSchema"],"properties":{**tool["inputSchema"]["properties"],"newOptionalFlag":{"type":"boolean"}}}} if tool["name"]=="read_document" else tool for tool in tools]
            if origin:
                tools = [{**tool,"inputSchema":{"type":"object","properties":{"value":{"$ref":origin+"/schema-external"}}}} if tool["name"]=="external_schema" else tool for tool in tools]
            offset = int(params.get("cursor", "0"))
            page = {"tools":tools[offset:offset+2]}
            if offset+2 < len(tools): page["nextCursor"]=str(offset+2)
            return page
        return {"tools": TOOLS}
    if method == "tools/call":
        if endpoint == "/mcp-rpc-error":
            raise ValueError("Synthetic upstream RPC failure")
        args = params.get("arguments", {})
        if params.get("name") in {"inspect_payload","odd_parameters","schema_secrets"} and endpoint in {"/mcp-tools","/mcp-slow","/mcp-generated","/mcp-generated-changed"}:
            output = {"received":args,"account":"synthetic-work-account"}
        elif params.get("name") == "search_documents":
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
            return self.send(200, {"resource": origin + "/mcp", "authorization_servers": [origin], "scopes_supported": ["documents:read","documents:write"] if self.server.registered else ["documents:read"]})
        if path.path.startswith("/.well-known/oauth-authorization-server") or path.path.startswith("/.well-known/openid-configuration"):
            metadata={"issuer": origin, "authorization_endpoint": origin + "/authorize", "token_endpoint": origin + "/token", "response_types_supported": ["code"], "grant_types_supported": ["authorization_code", "refresh_token"], "token_endpoint_auth_methods_supported": ["client_secret_"+self.server.token_auth] if self.server.registered else ["none"], "code_challenge_methods_supported": ["S256"], "scopes_supported": ["documents:read", "offline_access"], "authorization_response_iss_parameter_supported": True}
            if not self.server.registered: metadata["registration_endpoint"]=origin+"/register"
            return self.send(200,metadata)
        if path.path == "/authorize":
            query = {key: values[0] for key, values in parse_qs(path.query).items()}
            if self.server.registered and query.get("client_id")!="synthetic-registered-client":
                return self.send(400,{"error":"invalid_client"})
            self.server.requested_scopes=query.get("scope","").split()
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
            return self.send(200, {"refreshes": self.server.refreshes, "calls": self.server.calls,"lists":self.server.lists,"schemaReads":self.server.schema_reads,"sessionVersions":list(self.server.sessions.values()),"registrations":self.server.registrations,"tokenAuthChecks":self.server.token_auth_checks,"requestedScopes":self.server.requested_scopes})
        if path.path == "/schema-external":
            self.server.schema_reads += 1
            return self.send(200,{"type":"string"})
        return self.send(405, {"error": "POST required"})

    def do_POST(self):
        length = int(self.headers.get("Content-Length", "0"))
        raw = self.rfile.read(length)
        path = urlparse(self.path).path
        origin = f"http://127.0.0.1:{self.server.server_port}"
        if path == "/register":
            self.server.registrations+=1
            if self.server.registered: return self.send(400,{"error":"registration_not_supported"})
            body = json.loads(raw)
            return self.send(201, {"client_id": "mcplink-synthetic-client", "redirect_uris": body["redirect_uris"], "token_endpoint_auth_method": "none", "grant_types": ["authorization_code", "refresh_token"], "response_types": ["code"]})
        if path == "/token":
            body = {key: values[0] for key, values in parse_qs(raw.decode()).items()}
            if self.server.registered:
                client,secret=body.get("client_id"),body.get("client_secret")
                basic=self.headers.get("Authorization","").startswith("Basic ")
                if basic:
                    try: client,secret=base64.b64decode(self.headers["Authorization"][6:]).decode().split(":",1)
                    except (ValueError,UnicodeDecodeError): return self.send(400,{"error":"invalid_client"})
                self.server.token_auth_checks+=1
                if client!="synthetic-registered-client" or secret!="synthetic-registered-secret" or basic!=(self.server.token_auth=="basic"):
                    return self.send(400,{"error":"invalid_client"})
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
            if path == "/mcp-session":
                if request.get("method")=="initialize":
                    # Reproduce the observed remote behavior: the session is bound
                    # to the requested legacy version while the result negotiates
                    # 2025-11-25. Advertising 2026 in a legacy flow breaks it.
                    session=f"synthetic-session-{time.monotonic_ns()}"
                    self.server.sessions[session]=request["params"]["protocolVersion"]
                    result=handle(request,path,origin)|{"protocolVersion":"2025-11-25"}
                    data=("event: message\ndata: "+json.dumps({"jsonrpc":"2.0","id":request["id"],"result":result})+"\n\n").encode()
                    self.send_response(200);self.send_header("Content-Type","text/event-stream");self.send_header("Content-Length",str(len(data)));self.send_header("Mcp-Session-Id",session);self.end_headers();self.wfile.write(data)
                    return
                session=self.headers.get("Mcp-Session-Id","")
                if self.server.sessions.get(session)!=self.headers.get("MCP-Protocol-Version"):
                    return self.send(400,{"error":"invalid session"})
            if "id" not in request:
                return self.send(202, {})
            if request.get("method") == "tools/call":
                self.server.calls += 1
                if path == "/mcp-drop":
                    self.close_connection = True
                    self.connection.close()
                    return
            if request.get("method") == "tools/list":
                self.server.lists += 1
                if path == "/mcp-slow": time.sleep(0.8)
            result = handle(request, path, origin)
            return self.send(200, {"jsonrpc": "2.0", "id": request["id"], "result": result})
        except ValueError:
            return self.send(200, {"jsonrpc": "2.0", "id": request.get("id"), "error": {"code": -32601, "message": "Method not found"}})


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--stdio", action="store_true")
    parser.add_argument("--port", type=int, default=0)
    parser.add_argument("--auth", action="store_true")
    parser.add_argument("--registered-client", action="store_true")
    parser.add_argument("--token-auth", choices=["post","basic"], default="post")
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
    faulthandler.dump_traceback_later(10)
    try:
        server = LoopbackHTTPServer(("127.0.0.1", args.port), Handler)
    finally:
        faulthandler.cancel_dump_traceback_later()
    server.auth, server.codes, server.refreshes, server.calls = args.auth, {}, 0, 0
    server.lists, server.schema_reads = 0, 0
    server.sessions = {}
    server.registered, server.token_auth = args.registered_client, args.token_auth
    server.registrations, server.token_auth_checks, server.requested_scopes = 0, 0, []
    server.access_token, server.refresh_token = "unissued-synthetic-access", "unissued-synthetic-refresh"
    print(json.dumps({"port": server.server_port}), flush=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
