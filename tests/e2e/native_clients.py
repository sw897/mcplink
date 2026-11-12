"""Exercise vendor MCP runtimes against one managed entry and two independent upstreams.

No inference request, model account, or developer client configuration is used.
"""
import argparse
import hashlib
import json
from pathlib import Path
import queue
import subprocess
import sys
import threading
import time
import tomllib
import traceback

from agent_cli import AgentCli
from run import BINARY, FIXTURE, ROOT


class JsonLines:
    def __init__(self, command, cwd, environment, record):
        self.sequence = 0
        self.events = queue.Queue()
        self.log = (record / "native-stderr.log").open("w", encoding="utf-8")
        self.process = subprocess.Popen(command, cwd=cwd, env=environment,
                                        stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                        stderr=self.log, text=True, encoding="utf-8")

        def read():
            try:
                for line in self.process.stdout:
                    self.events.put(json.loads(line))
            except Exception as error:
                self.events.put(error)
            finally:
                self.events.put(None)
        threading.Thread(target=read, daemon=True).start()

    def notify(self, method):
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method}) + "\n")
        self.process.stdin.flush()

    def request(self, method, params):
        self.sequence += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.sequence,
                                            "method": method, "params": params}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 60
        while time.monotonic() < deadline:
            message = self.events.get(timeout=max(.1, deadline - time.monotonic()))
            if message is None or isinstance(message, Exception):
                raise AssertionError("Native runtime closed or emitted invalid JSON; see private stderr log")
            if message.get("id") == self.sequence:
                return message
            assert "id" not in message, "Unexpected native runtime request"
        raise TimeoutError(method)

    def result(self, method, params):
        response = self.request(method, params)
        assert "error" not in response, response
        return response["result"]

    def close(self):
        if self.process.poll() is None:
            self.process.stdin.close()
            try:
                self.process.wait(timeout=15)
            except subprocess.TimeoutExpired:
                self.process.terminate()
                self.process.wait(timeout=10)
        self.log.close()


class Codex(JsonLines):
    name = "Codex"
    refresh_mode = "explicit native MCP reload and a new thread"

    def __init__(self, binary, cwd, environment, record):
        super().__init__([str(binary), "app-server", "--stdio", "-c", 'model_provider="fixture"',
                          "-c", 'model_providers.fixture={name="Fixture",base_url="http://127.0.0.1:9",wire_api="responses",requires_openai_auth=false}',
                          "-c", "analytics.enabled=false"], cwd, environment, record)
        self.result("initialize", {"clientInfo": {"name": "mcplink-acceptance", "version": "1"},
                                   "capabilities": {"experimentalApi": True}})
        self.notify("initialized")
        self.project = cwd
        self.start_thread()

    def start_thread(self):
        self.thread = self.result("thread/start", {"cwd": str(self.project), "ephemeral": True,
                                                   "sandbox": "read-only", "approvalPolicy": "on-request"})["thread"]["id"]

    def tools(self):
        value = self.result("mcpServerStatus/list", {"threadId": self.thread, "serverName": "mcplink",
                                                     "detail": "toolsAndAuthOnly"})
        assert len(value["data"]) == 1 and value["data"][0]["name"] == "mcplink", value
        return set(value["data"][0]["tools"])

    def call(self, tool):
        return self.request("mcpServer/tool/call", {"threadId": self.thread, "server": "mcplink",
                                                   "tool": tool, "arguments": {"id": "weekly-fixture"}})

    def refresh(self):
        self.result("config/mcpServer/reload", {})
        # Codex keeps an existing thread's MCP inventory until its next turn.
        # A fresh ephemeral thread exercises native reload without model inference.
        self.start_thread()


class Hermes(JsonLines):
    name = "Hermes"
    refresh_mode = "live catalog notification"

    def __init__(self, python, source, cwd, environment, record):
        super().__init__([str(python), str(ROOT / "tests/fixtures/hermes-native-mcp.py"), str(source)],
                         cwd, environment, record)
        self.result("initialize", {})

    def tools(self):
        # Hermes prefixes its registered tool names with the native server key.
        names = set(self.result("tools/list", {}))
        helpers = {"mcp__mcplink__" + name for name in ["get_prompt", "list_prompts", "list_resources", "read_resource"]}
        assert all(name in helpers or name.startswith("mcp__mcplink__s_") for name in names), names
        return names - helpers

    def call(self, tool):
        return self.request("tools/call", {"name": tool, "arguments": {"id": "weekly-fixture"}})

    def refresh(self):
        pass


def wait_tools(client, expected_suffixes):
    deadline = time.monotonic() + 25
    while True:
        tools = client.tools()
        if len(tools) == len(expected_suffixes) and all(any(t.endswith(s) for t in tools) for s in expected_suffixes):
            return tools
        assert time.monotonic() < deadline, (client.name, sorted(tools), expected_suffixes)
        time.sleep(.25)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--codex", type=Path)
    parser.add_argument("--hermes-source", type=Path)
    parser.add_argument("--hermes-python", type=Path)
    args = parser.parse_args()
    assert bool(args.hermes_source) == bool(args.hermes_python), "Hermes requires both its pinned source and its Python runtime"
    assert args.codex or (args.hermes_source and args.hermes_python), "Select at least one actual vendor runtime"
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = AgentCli(record)
    clients = []
    conclusion = "FAIL"
    try:
        # Presence launchers in Harness only support source discovery. The actual
        # vendor processes below always use explicit, separately verified binaries.
        codex_config = harness.home / ".codex/config.toml"
        hermes_config = harness.home / ".hermes/config.yaml"
        codex_config.write_text('model = "fixture-model"\n', encoding="utf-8")
        hermes_config.write_text('model:\n  default: fixture-model\nmcp_servers: {}\n', encoding="utf-8")
        first = harness.rpc("connections.save", {"name": "native_http", "definition": {
            "url": harness.url, "headers": {"Authorization": "synthetic-fixture-authorization"}}})["id"]
        second = harness.rpc("connections.save", {"name": "native_stdio", "definition": {
            "command": sys.executable, "args": [str(FIXTURE), "--stdio"]}})["id"]
        entry = harness.rpc("gateway.save", {"title": "Native acceptance", "connectionIds": [first, second]})
        harness.rpc("connections.probe", {"id": entry["id"]})
        targets = (["codex:user"] if args.codex else []) + (["hermes:user"] if args.hermes_source else [])
        _, applied = harness.changes(entry["id"], targets, mode="shared")
        configs = {p: p.read_bytes() for p in [codex_config, hermes_config]}
        if args.codex:
            assert set(tomllib.loads(codex_config.read_text())["mcp_servers"]) == {"mcplink"}
        assert all(b"synthetic-fixture-authorization" not in value for value in configs.values())
        harness.passed("native configurations contain one mcplink entry and no upstream credentials")
        environment = {k: v for k, v in harness.environment.items()
                       if not any(s in k.upper() for s in ["TOKEN", "API_KEY", "SECRET"])}
        environment.update(NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost")
        project = record / "project"
        project.mkdir()
        for kind, options in [(Codex, [args.codex]), (Hermes, [args.hermes_python, args.hermes_source])]:
            if not options[0]:
                continue
            client_record = record / kind.name.lower()
            client_record.mkdir()
            client = kind(*[p.absolute() for p in options], project, environment, client_record)
            clients.append(client)
        namespaces = {m["connectionId"]: m["namespace"] for m in entry["members"]}
        suffixes = [namespaces[member] + "__" + tool for member in [first, second]
                    for tool in ["read_document", "search_documents"]]
        for client in clients:
            tools = wait_tools(client, suffixes)
            for member in [first, second]:
                name = next(t for t in tools if t.endswith(namespaces[member] + "__read_document"))
                response = client.call(name)
                assert "error" not in response and not response["result"].get("isError"), response
                assert "weekly-fixture" in json.dumps(response), response
            harness.passed(f"{client.name} native MCP discovery and calls reach two independent upstreams through one entry without inference")
        removed = {}
        for client in clients:
            removed[client] = next(t for t in client.tools() if t.endswith(namespaces[second] + "__read_document"))
        entry = harness.rpc("gateway.save", {"id": entry["id"], "editVersion": entry["editVersion"], "connectionIds": [first]})
        for client in clients:
            client.refresh()
            wait_tools(client, [s for s in suffixes if s.startswith(namespaces[first] + "__")])
            response = client.call(removed[client])
            assert "error" in response or response["result"].get("isError") or "error" in json.dumps(response["result"]).lower(), response
            harness.passed(f"{client.name} updates selection through {client.refresh_mode} and refuses a removed route")
        entry = harness.rpc("gateway.save", {"id": entry["id"], "editVersion": entry["editVersion"], "connectionIds": [first, second]})
        for client in clients:
            client.refresh()
            wait_tools(client, suffixes)
        harness.rpc("credentials.update", {"id": first, "value": "synthetic-invalid-authorization"})
        for client in clients:
            client.refresh()
            tools = wait_tools(client, [s for s in suffixes if s.startswith(namespaces[second] + "__")])
            response = client.call(next(t for t in tools if t.endswith("__read_document")))
            assert "error" not in response and not response["result"].get("isError"), response
            harness.passed(f"{client.name} keeps the healthy upstream usable when its peer loses authorization")
        harness.rpc("credentials.update", {"id": first, "value": "synthetic-fixture-authorization"})
        for client in clients:
            client.refresh()
            tools = wait_tools(client, suffixes)
            response = client.call(next(t for t in tools if t.endswith(namespaces[first] + "__read_document")))
            assert "error" not in response and not response["result"].get("isError"), response
        assert all(p.read_bytes() == value for p, value in configs.items())
        harness.passed("one credential update restores every native runtime without rewriting its configuration")
        for client in clients:
            client.close()
        clients.clear()
        for row in applied["results"]:
            assert harness.rpc("changes.restore", {"operationId": row["operationId"]})["status"] == "restored"
        assert not tomllib.loads(codex_config.read_text()).get("mcp_servers")
        assert "mcplink:" not in hermes_config.read_text()
        harness.passed("native entry restoration preserves unrelated model settings and removes only the managed entry")
        conclusion = "PASS"
    except Exception:
        traceback.print_exc()
    finally:
        for client in clients:
            client.close()
        harness.close(False)
        (record / "report.json").write_text(json.dumps({"conclusion": conclusion,
            "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(), "scenarios": harness.results,
            "model_account_used": False, "personal_configuration_modified": False}, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
