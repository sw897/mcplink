"""Run mcplink through its CLI, bridge and native Agent configuration entries."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import http.client
import json
import os
from pathlib import Path
import queue
import signal
import shutil
import subprocess
import sys
import threading
import time
import tomllib
import traceback
import urllib.error
import urllib.parse
import urllib.request


ROOT = Path(__file__).resolve().parents[2]
BINARY = Path(os.environ.get("MCPLINK_E2E_BINARY", ROOT / "target/debug" / ("mcplink.exe" if os.name == "nt" else "mcplink"))).resolve()
FIXTURE = ROOT / "tests/fixtures/mcp_server.py"


class McpClient:
    def __init__(self, connection, environment, label):
        self.process = subprocess.Popen([str(BINARY), "bridge", "--connection", connection], env=environment, stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, bufsize=1)
        self.responses = queue.Queue()
        def read_responses():
            for line in self.process.stdout:
                self.responses.put(line)
            self.responses.put(None)
        threading.Thread(target=read_responses, daemon=True).start()
        self.sequence = 0
        self.call("initialize", {"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": label, "version": "1.0.0"}})
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "method": "notifications/initialized"}) + "\n")
        self.process.stdin.flush()

    def call(self, method, params=None):
        self.sequence += 1
        self.process.stdin.write(json.dumps({"jsonrpc": "2.0", "id": self.sequence, "method": method, "params": params or {}}) + "\n")
        self.process.stdin.flush()
        deadline = time.monotonic() + 45
        while time.monotonic() < deadline:
            try:
                line = self.responses.get(timeout=1)
            except queue.Empty:
                if self.process.poll() is not None:
                    raise AssertionError("Bridge exited: " + self.process.stderr.read()[:400])
                continue
            if not line:
                raise AssertionError("Bridge closed its output")
            response = json.loads(line)
            if response.get("id") == self.sequence:
                return response
        raise AssertionError("Bridge response timed out")

    def close(self):
        self.process.stdin.close()
        try:
            self.process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            self.process.terminate()
            self.process.wait(timeout=5)


class Harness:
    def __init__(self, record, native_agents=False, fixture_args=()):
        self.record = record
        self.native_agents = native_agents
        self.home = record / "home"
        self.data = record / "data"
        self.results = []
        self.clients = []
        self.environment = {**os.environ, "MCPLINK_HOME": str(self.home), "MCPLINK_DATA_DIR": str(self.data), "MCPLINK_VAULT": "fixture", "CODEX_HOME": str(self.home / ".codex"), "HERMES_HOME": str(self.home / ".hermes"), "CLAUDE_CONFIG_DIR": str(self.home / ".claude")}
        for path in (self.home / ".codex", self.home / ".hermes"):
            path.mkdir(parents=True)
        (self.home / ".mcplink-test-fixture").touch()
        self.server = subprocess.Popen([sys.executable, str(FIXTURE), "--auth", *fixture_args], stdout=subprocess.PIPE, stderr=(record / "fixture.log").open("w"), text=True)
        startup = queue.Queue()
        threading.Thread(target=lambda: startup.put(self.server.stdout.readline()), daemon=True).start()
        try:
            self.port = json.loads(startup.get(timeout=20))["port"]
        except (queue.Empty, json.JSONDecodeError, KeyError) as error:
            self.server.terminate()
            self.server.wait(timeout=5)
            diagnostic = (record / "fixture.log").read_text(errors="replace")[-4000:]
            raise AssertionError(f"Synthetic MCP fixture did not announce a port; see {record / 'fixture.log'}\n{diagnostic}") from error
        print("READY synthetic MCP fixture", flush=True)
        self.url = f"http://127.0.0.1:{self.port}/mcp"
        local = {"command": sys.executable, "args": [str(FIXTURE), "--stdio"]}
        toml = '# Codex user comment\nmodel = "fixture-model" # preserve this\n\n[mcp_servers.local_notes]\ncommand = ' + json.dumps(sys.executable) + '\nargs = ' + json.dumps(local["args"]) + '\n'
        (self.home / ".codex/config.toml").write_text(toml)
        yaml = '# Hermes user comment\nmodel:\n  default: fixture-model\nmcp_servers:\n  office_docs:\n    url: ' + self.url + '\n    headers:\n      Authorization: synthetic-fixture-authorization\n  disabled_tool:\n    command: ' + json.dumps(sys.executable) + '\n    args: ' + json.dumps(local["args"]) + '\n    enabled: false\n'
        (self.home / ".hermes/config.yaml").write_text(yaml)

    def rpc(self, method, params=None, error=False):
        result = subprocess.run([str(BINARY), "rpc", "--stdin"], input=json.dumps({"method": method, "params": params or {}}), env=self.environment, capture_output=True, text=True, timeout=90)
        value = json.loads(result.stdout)
        if error:
            assert "error" in value, (method, value)
            return value["error"]["message"]
        assert "error" not in value, (method, value)
        return value["ok"]

    def passed(self, name, evidence=None):
        self.results.append({"scenario": name, "result": "PASS", "evidence": evidence or {}})
        print("PASS", name, flush=True)

    def bridge(self, connection, label):
        client = McpClient(connection, self.environment, label)
        self.clients.append(client)
        return client

    def changes(self, connection, targets, mode="direct", action="upsert"):
        plan = self.rpc("changes.preview", {"connectionId": connection, "targetSourceIds": targets, "mode": mode, "action": action})
        result = self.rpc("changes.apply", {"planId": plan["planId"]})
        assert all(row["status"] == "applied" for row in result["results"]), result
        return plan, result

    def run(self):
        snapshot = self.rpc("snapshot")
        assert len(snapshot["connections"]) == 3
        assert len(snapshot["sources"]) == 3
        assert next(binding for binding in snapshot["bindings"] if binding["name"] == "disabled_tool")["enabled"] is False
        assert next(c for c in snapshot["connections"] if c["name"]=="disabled_tool")["authKind"]=="none"
        self.passed("scan native sources and disabled service")
        proxy_env={**self.environment,"HTTP_PROXY":"http://127.0.0.1:9","http_proxy":"http://127.0.0.1:9","HTTPS_PROXY":"http://127.0.0.1:9","NO_PROXY":"","no_proxy":""}
        proxy_result=subprocess.run([str(BINARY),"rpc","snapshot"],env=proxy_env,capture_output=True,text=True,timeout=8)
        assert "ok" in json.loads(proxy_result.stdout),proxy_result.stdout
        self.passed("local authenticated control traffic bypasses proxies")

        local = self.rpc("connections.import", {"sourceId": "codex:user", "name": "local_notes"})["id"]
        office = self.rpc("connections.import", {"sourceId": "hermes:user", "name": "office_docs"})["id"]
        for connection in (local, office):
            probe = self.rpc("connections.probe", {"id": connection})
            assert probe["connection"]["status"] == "ready", probe
            assert {tool["name"] for tool in probe["tools"]} == {"search_documents", "read_document"}
        self.passed("real SDK discovers stdio and authenticated HTTP fixture")

        invalid = self.rpc("connections.save", {"name": "invalid_grant", "definition": {"url": self.url, "enabled": True, "headers": {"Authorization": "synthetic-invalid-authorization"}}})["id"]
        rejected = self.rpc("connections.probe", {"id": invalid})
        assert rejected["connection"]["status"] == "needs_auth", "HTTP 401 must request authentication rather than report a generic connection error"
        self.passed("HTTP 401 and SDK AuthRequired are classified as needs_auth")

        no_challenge = self.rpc("connections.save", {"name": "invalid_no_challenge", "definition": {"url": self.url + "-no-challenge", "enabled": True, "headers": {"Authorization": "synthetic-invalid-authorization"}}})["id"]
        rejected = self.rpc("connections.probe", {"id": no_challenge})
        assert rejected["connection"]["status"] == "needs_auth", "HTTP 401 inside discover/initialize fallback must remain an authentication failure"
        self.passed("HTTP 401 inside SDK legacy fallback is classified as needs_auth")

        _, local_result = self.changes(local, ["hermes:user"])
        _, office_result = self.changes(office, ["codex:user"])
        codex = (self.home / ".codex/config.toml").read_text()
        hermes = (self.home / ".hermes/config.yaml").read_text()
        assert "# Codex user comment" in codex and '# preserve this' in codex
        assert "# Hermes user comment" in hermes and "fixture-model" in hermes
        assert "office_docs" in tomllib.loads(codex)["mcp_servers"]
        self.passed("bidirectional config writes preserve unrelated content")

        plan = self.rpc("changes.preview", {"connectionId": local, "targetSourceIds": ["hermes:user"], "action": "disable"})
        hermes_path = self.home / ".hermes/config.yaml"
        hermes_path.write_text(hermes_path.read_text() + "\n# later unrelated edit\n")
        blocked = self.rpc("changes.apply", {"planId": plan["planId"]})
        assert blocked["results"][0]["status"] == "failed"
        self.passed("preview rejects concurrent file changes")

        restored = self.rpc("changes.restore", {"operationId": local_result["results"][0]["operationId"]})
        assert restored["status"] == "restored"
        assert "# later unrelated edit" in hermes_path.read_text()
        self.changes(local, ["hermes:user"])
        self.passed("entry restore preserves later unrelated edits")

        self.changes(office, ["codex:user", "hermes:user"], mode="shared")
        for path in (self.home / ".codex/config.toml", self.home / ".hermes/config.yaml"):
            assert "synthetic-fixture-authorization" not in path.read_text()
        database = b"".join(path.read_bytes() for path in self.data.glob("mcplink.sqlite3*"))
        assert b"synthetic-fixture-authorization" not in database
        self.passed("shared configs and SQLite contain credential references")
        alternative=self.record/f"alternate-cli{BINARY.suffix}";shutil.copy2(BINARY,alternative)
        result=subprocess.run([str(alternative),"rpc","--stdin"],env=self.environment,input=json.dumps({"method":"changes.preview","params":{"connectionId":office,"targetSourceIds":["codex:user"],"mode":"shared"}}),text=True,capture_output=True,timeout=20)
        alternate_plan=json.loads(result.stdout)["ok"]
        assert alternate_plan["changes"][0]["after"]["command"]==str(alternative)
        self.passed("shared entry uses invoking application path across daemon locations")


        a, b = self.bridge(office, "codex-transport-probe"), self.bridge(office, "hermes-transport-probe")
        for client in (a, b):
            listed = client.call("tools/list")["result"]["tools"]
            assert {tool["name"] for tool in listed} == {"search_documents", "read_document"}
            assert "result" in client.call("resources/list")
            assert "result" in client.call("resources/read", {"uri": "fixture://weekly"})
            assert "result" in client.call("prompts/list")
            assert "result" in client.call("prompts/get", {"name": "weekly_report"})
        with ThreadPoolExecutor(max_workers=2) as pool:
            responses = list(pool.map(lambda client: client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture", "delay_ms": 150}}), (a, b)))
        contents = [json.loads(response["result"]["content"][0]["text"]) for response in responses]
        assert all(content["workspace"] == "fixture-workspace" and content["account"] == "synthetic-work-account" and content["text"] == contents[0]["text"] for content in contents)
        self.passed("two bridges share account, tools, resources and prompts")

        self.rpc("gateway.pause")
        assert "error" in a.call("tools/list")
        assert self.rpc("connections.probe", {"id": local})["connection"]["status"] == "ready"
        self.rpc("gateway.resume")
        assert "result" in b.call("tools/list")
        self.passed("paused gateway and independent direct connection")

        self.rpc("credentials.disconnect", {"id": office})
        assert "error" in a.call("tools/list")
        self.rpc("credentials.update", {"id": office, "value": "synthetic-fixture-authorization"})
        assert "result" in b.call("tools/list")
        self.passed("credential disconnect and update affect both bridges")

        oauth = self.rpc("connections.save", {"name": "oauth_docs", "title": "OAuth fixture", "definition": {"url": self.url, "enabled": True, "oauth": True}})["id"]
        authorization = self.rpc("oauth.start", {"id": oauth})
        with urllib.request.urlopen(authorization["url"], timeout=20) as response:
            assert response.status == 200
        assert self.rpc("connections.probe", {"id": oauth})["connection"]["status"] == "ready"
        oauth_client = self.bridge(oauth, "oauth-transport-probe")
        time.sleep(4)
        assert "result" in oauth_client.call("tools/list")
        with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/stats") as response:
            stats = json.load(response)
        assert stats["refreshes"] >= 1
        self.passed("OAuth PKCE callback, persistence and token refresh", {"refreshes": stats["refreshes"]})

        runtime = json.loads((self.data / "runtime.json").read_text())
        for headers in ({"Host": "evil.example"}, {"Origin": "https://evil.example"}, {}):
            connection = http.client.HTTPConnection("127.0.0.1", runtime["port"], timeout=5)
            connection.request("POST", "/rpc", body=json.dumps({"method": "snapshot"}), headers={"Content-Type": "application/json", **headers})
            assert connection.getresponse().status == 403
            connection.close()
        self.passed("local API rejects invalid Host, Origin and credentials")

        if self.native_agents:
            for program, args in (("codex", ["mcp", "list", "--json"]), ("hermes", ["mcp", "test", "office_docs"])):
                result = subprocess.run([program, *args], env=self.environment, capture_output=True, text=True, timeout=90)
                (self.record / f"{program}-native.log").write_text(result.stdout + result.stderr)
                assert result.returncode == 0, (program, result.stdout[-400:], result.stderr[-400:])
                self.passed(f"native {program} MCP entry", {"exit_status": result.returncode})
        # Review regressions exercise the same public CLI/bridge paths.
        batch=self.rpc("changes.preview",{"connectionIds":[local,office],"targetSourceIds":["codex:user","hermes:user"],"action":"disable"})
        result=self.rpc("changes.apply",{"planId":batch["planId"]})
        assert len(result["results"])==4 and all(r["status"]=="applied" for r in result["results"]),result
        for target in ("codex:user","hermes:user"):
            self.changes(local,[target],action="enable")
            self.changes(office,[target],action="enable")
        assert "重复" in self.rpc("changes.preview",{"connectionIds":[local,local],"targetSourceIds":["codex:user"]},error=True)
        self.passed("batch same-file operations and duplicate rejection")

        stale=self.rpc("changes.preview",{"connectionId":local,"targetSourceIds":["hermes:user"]})
        detail=self.rpc("connections.detail",{"id":local})
        self.rpc("connections.save",{"id":local,"name":"local_notes","definition":detail["definition"]})
        failed=self.rpc("changes.apply",{"planId":stale["planId"]})
        assert failed["results"][0]["status"]=="failed"
        assert any(b.get("needsUpdate") for b in self.rpc("snapshot")["bindings"] if b.get("connectionId")==local)
        self.changes(local,["codex:user","hermes:user"])
        self.passed("stale connection preview rejected and direct bindings need synchronization")

        partial=self.rpc("changes.preview",{"connectionId":local,"targetSourceIds":["codex:user","hermes:user"],"action":"disable"})
        hermes_path.write_text(hermes_path.read_text()+"\n# second external edit\n")
        result=self.rpc("changes.apply",{"planId":partial["planId"]})
        assert [r["status"] for r in result["results"]]==["applied","failed"],result
        self.changes(local,["codex:user"],action="enable")
        self.passed("independent target succeeds when another target changes")
        _,disable_result=self.changes(local,["codex:user"],action="disable")
        codex_path=self.home/".codex/config.toml"
        codex_path.write_text(codex_path.read_text().replace("enabled = false","enabled = true"))
        assert "后续修改" in self.rpc("changes.restore",{"operationId":disable_result["results"][0]["operationId"]},error=True)
        self.rpc("connections.adopt_source",{"sourceId":"codex:user","name":"local_notes"})
        self.passed("restore refuses same-entry edits and adoption resolves divergence")


        external=self.record/"custom.yaml"
        external.write_text('mcp_servers:\n  filtered:\n    url: '+self.url+'\n    headers:\n      Authorization: synthetic-fixture-authorization\n    tools:\n      include: ["read*"]\n      exclude: [read_document]\n      resources: false\n  broken: wrong-format\n')
        source=self.rpc("sources.add",{"agent":"hermes","path":str(external)})["id"]
        assert self.rpc("sources.add",{"agent":"hermes","path":str(external)})["id"]==source
        snapshot=self.rpc("snapshot")
        assert any(c["name"]=="broken" and c["status"]=="error" for c in snapshot["connections"])
        filtered=self.rpc("connections.import",{"sourceId":source,"name":"filtered"})["id"]
        result=self.rpc("connections.probe",{"id":filtered})
        assert [t["name"] for t in result["tools"]]==["read_document"],result
        assert "筛选" in self.rpc("changes.preview",{"connectionId":filtered,"targetSourceIds":["codex:user"]},error=True)
        self.changes(filtered,["codex:user"],mode="shared")
        filtered_client=self.bridge(filtered,"filter-regression")
        assert "error" in filtered_client.call("tools/call",{"name":"search_documents","arguments":{"query":"42"}})
        assert "error" in filtered_client.call("resources/list")
        self.passed("invalid entry isolation and Hermes filter preservation")

        old=external.read_text()
        external.write_text(old.replace('include: ["read*"]','include: []'))
        self.rpc("connections.adopt_source",{"sourceId":source,"name":"filtered"})
        result=self.rpc("connections.probe",{"id":filtered})
        assert result["tools"]==[],result
        self.passed("adopt native changes and explicit empty tool whitelist")

        bad=self.rpc("oauth.start",{"id":oauth})
        query=urllib.parse.parse_qs(urllib.parse.urlsplit(bad["url"]).query)
        callback=query["redirect_uri"][0]
        try:
            urllib.request.urlopen(callback+"?code=invalid&state=wrong",timeout=10)
            raise AssertionError("Invalid OAuth state accepted")
        except urllib.error.HTTPError as err:
            assert err.code==400
        self.rpc("credentials.disconnect",{"id":oauth})
        try:
            urllib.request.urlopen(callback+"?code=invalid&state="+query["state"][0],timeout=10)
            raise AssertionError("Callback after disconnect accepted")
        except urllib.error.HTTPError as err:
            assert err.code==400
        self.passed("OAuth invalid state and callback after disconnect rejected")

        # Stop our synthetic daemon, inject process exit after file replacement, then reconcile.
        runtime=json.loads((self.data/"runtime.json").read_text())
        os.kill(runtime["pid"],signal.SIGTERM)
        time.sleep(0.3)
        self.environment["MCPLINK_FAILPOINT"]="after-write"
        interrupted=self.rpc("changes.preview",{"connectionId":local,"targetSourceIds":["hermes:user"],"action":"disable"})
        result=self.rpc("changes.apply",{"planId":interrupted["planId"]},error=True)
        del self.environment["MCPLINK_FAILPOINT"]
        snapshot=self.rpc("snapshot")
        assert any(op["id"]==interrupted["changes"][0]["id"] and op["status"]=="applied" for op in snapshot["history"])
        assert "result" in a.call("tools/list")
        self.passed("post-write crash reconciles and existing bridge reconnects")
        codex_path.write_text(codex_path.read_text().replace("mcp_servers.local_notes","mcp_servers.external_renamed"))
        snapshot=self.rpc("snapshot")
        assert any(b.get("missing") and b.get("connectionId")==local for b in snapshot["bindings"])
        self.changes(local,["codex:user"],action="remove")
        assert not any(b.get("connectionId")==local and b["sourceId"]=="codex:user" for b in self.rpc("snapshot")["bindings"])
        self.passed("removed native entry is visible and stale binding can be cleaned")

        return {"local": local, "office": office, "oauth": oauth}

    def close(self, keep):
        for client in self.clients:
            client.close()
        if not keep:
            try:
                runtime = json.loads((self.data / "runtime.json").read_text())
                os.kill(runtime["pid"], signal.SIGTERM)
            except (FileNotFoundError, ProcessLookupError):
                pass
            self.server.terminate()
            self.server.wait(timeout=10)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path)
    parser.add_argument("--keep", action="store_true")
    parser.add_argument("--native-agents", action="store_true", help="Also exercise installed Codex and Hermes CLI commands")
    args = parser.parse_args()
    record = args.record or ROOT / "artifacts/e2e/core" / datetime.now().astimezone().strftime("%Y%m%dT%H%M%S%z")
    record = record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    print("START isolated core acceptance", flush=True)
    harness = Harness(record, native_agents=args.native_agents)
    conclusion, error, identifiers = "PASS", None, {}
    try:
        identifiers = harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        print("FAIL", error, flush=True)
        traceback.print_exc()
    finally:
        harness.close(args.keep)
    report = {"conclusion": conclusion, "code_version": json.loads((ROOT/"package.json").read_text())["version"], "native_agents_requested": args.native_agents, "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(), "os": __import__("platform").platform(), "cargo_lock_sha256": hashlib.sha256((ROOT/"Cargo.lock").read_bytes()).hexdigest(), "package_lock_sha256": hashlib.sha256((ROOT/"package-lock.json").read_bytes()).hexdigest(), "executed_at": datetime.now(timezone.utc).isoformat(), "command": "python3 tests/e2e/run.py", "fixture": "synthetic MCP and OAuth", "scenarios": harness.results, "error": error, "ids": identifiers, "home": str(harness.home), "data": str(harness.data), "fixture_port": harness.port, "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(), "skill_sha256": hashlib.sha256((ROOT / "tests/fixtures/weekly-report/SKILL.md").read_bytes()).hexdigest()}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(record)
    if args.keep and conclusion=="PASS":
        print("Keeping synthetic fixture alive for desktop/native verification",flush=True)
        try:
            while True: time.sleep(30)
        except KeyboardInterrupt: harness.close(False)
    raise SystemExit(0 if conclusion == "PASS" else 1)


if __name__ == "__main__":
    main()
