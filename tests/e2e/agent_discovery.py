"""Exercise native Agent formats, import-only inventory and actual shared MCP commands."""
import argparse
from datetime import datetime, timezone
import hashlib
from http.server import BaseHTTPRequestHandler, HTTPServer
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import threading
import tomllib
import traceback

from run import BINARY, FIXTURE, ROOT, Harness, McpClient

NATIVE = {"codex", "hermes", "claude", "opencode", "zcode", "deepseek", "openclaw", "kimicode", "workbuddy", "qwenwork", "pi"}
MANUAL = {"doubaowork"}
SUPPORTED = NATIVE | MANUAL
REMOVED = {"trae", "traecn", "vscode"}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = Harness(record, installed_agents=False)
    results = []

    def passed(name, evidence=None):
        results.append({"scenario": name, "result": "PASS", "evidence": evidence or {}})
        print("PASS", name, flush=True)

    def snapshot():
        value = harness.rpc("snapshot")
        assert {a["id"] for a in value["agents"]} == SUPPORTED
        assert all(a["management"] == ("manual" if a["id"] in MANUAL else "writable") for a in value["agents"])
        assert all(a["configurable"] == (a["installed"] or a["id"] in MANUAL) for a in value["agents"])
        return value

    def native_entry(source, name):
        path = Path(source["path"])
        if source["agent"] == "codex":
            return tomllib.loads(path.read_text(encoding="utf-8"))["mcp_servers"][name]
        command = ["node", str(ROOT / "tests/fixtures/read-native-config.mjs"), source["agent"], str(path), name]
        result = subprocess.run(command, capture_output=True, text=True, encoding="utf-8", check=True, timeout=20)
        return json.loads(result.stdout)

    def native_client(source, name, connection):
        client = McpClient(connection, harness.environment, "native-" + source["agent"], native_entry(source, name))
        harness.clients.append(client)
        tools = client.call("tools/list")["result"]["tools"]
        assert {t["name"] for t in tools} == {"search_documents", "read_document"}
        result = client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
        content = json.loads(result["result"]["content"][0]["text"])
        assert content["workspace"] == "fixture-workspace"
        return client, content

    conclusion, error = "PASS", None
    try:
        initial = snapshot()
        assert not any(Path(s["path"]).exists() for s in initial["sources"] if s["agent"] in {"openclaw", "kimicode", "zcode", "deepseek"})
        assert not next(a for a in initial["agents"] if a["id"] == "kimicode")["detected"]
        assert not next(a for a in initial["agents"] if a["id"] == "openclaw")["detected"]
        assert {s["agent"] for s in initial["sources"] if not s["readOnly"]} == NATIVE
        assert not any(a["installed"] or a["configurable"] for a in initial["agents"] if a["id"] in NATIVE)
        assert all(a["configurable"] and a["profilePath"] is None for a in initial["agents"] if a["id"] in MANUAL)
        for agent in NATIVE:
            assert "未安装" in harness.rpc("sources.add", {"agent":agent}, error=True)
        local = harness.rpc("connections.import", {"sourceId":"codex:user", "name":"local_notes"})["id"]
        for source in initial["sources"]:
            assert "未安装" in harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":[source["id"]]}, error=True)
        assert not any(Path(s["path"]).exists() for s in initial["sources"] if s["agent"] in {"openclaw", "kimicode", "zcode", "deepseek"})
        passed("native clients remain visible but cannot write when uninstalled; manual import clients can prepare configuration offline")
        harness.install_clients()
        assert all(a["installed"] and a["configurable"] for a in snapshot()["agents"] if a["id"] in NATIVE)
        passed("refreshing after installation enables configuration without creating native config files")

        # Retired clients and old Kimi CLI directories must not become active integrations.
        retired = harness.home / "AppData/Roaming/Code/User/mcp.json"
        retired.parent.mkdir(parents=True)
        retired.write_text('{"servers":{"retired":{"command":"never-run"}}}', encoding="utf-8")
        old_kimi = harness.home / ".kimi/mcp.json"
        old_kimi.parent.mkdir()
        old_kimi.write_text('{"mcpServers":{"old_kimi":{"command":"never-run"}}}', encoding="utf-8")
        with sqlite3.connect(harness.data / "mcplink.sqlite3") as db:
            db.execute("INSERT INTO sources(id,document) VALUES(?,?)", ("retired", json.dumps({"id":"retired","agent":"vscode","label":"retired","path":str(retired),"project":None,"readOnly":True})))
        retired_status = snapshot()
        assert not any(s["agent"] in REMOVED for s in retired_status["sources"])
        assert not any(c["name"] in {"retired", "old_kimi"} for c in retired_status["connections"])
        for agent in REMOVED:
            assert "支持名单" in harness.rpc("sources.add", {"agent":agent,"path":str(retired)}, error=True)
        passed("removed integrations and persisted retired metadata stay inactive; unrelated native files remain intact")

        sources = [s for s in snapshot()["sources"] if not s["readOnly"]]
        office = harness.rpc("connections.import", {"sourceId":"hermes:user", "name":"office_docs"})["id"]
        assert harness.rpc("connections.probe", {"id":office})["connection"]["status"] == "ready"
        openclaw = next(s for s in sources if s["agent"] == "openclaw")
        openclaw_path = Path(openclaw["path"])
        openclaw_path.parent.mkdir(parents=True)
        openclaw_path.write_text("// retain OpenClaw comment\n{ models: { mode: 'merge', keep: 0x20 }, mcp: { servers: {} } }\n", encoding="utf-8")
        dsh = next(s for s in sources if s["agent"] == "deepseek")
        dsh_path = Path(dsh["path"])
        dsh_path.parent.mkdir(parents=True)
        dsh_unrelated = "# retain DSH comment\n- id: unrelated\n  config:\n    native: !!js process.env.UNCHANGED\n"
        dsh_path.write_text(dsh_unrelated, encoding="utf-8")
        before_preview = {s["id"]: Path(s["path"]).read_bytes() if Path(s["path"]).exists() else None for s in sources}
        plan = harness.rpc("changes.preview", {"connectionId":office,"targetSourceIds":[s["id"] for s in sources],"mode":"shared"})
        assert before_preview == {s["id"]: Path(s["path"]).read_bytes() if Path(s["path"]).exists() else None for s in sources}
        applied = harness.rpc("changes.apply", {"planId":plan["planId"]})
        assert all(r["status"] == "applied" for r in applied["results"]), applied
        passed("one read-only preview and transactional batch applies shared native MCP entries to eleven clients and DSH profiles")

        shared = snapshot()
        bindings = [b for b in shared["bindings"] if b.get("connectionId") == office]
        assert {b["agent"] for b in bindings} == NATIVE
        assert all(b["mode"] == "shared" and not b["changed"] for b in bindings), bindings
        for source in sources:
            assert "synthetic-fixture-authorization" not in Path(source["path"]).read_text(encoding="utf-8")
            client, content = native_client(source, "office_docs", office)
            assert content["account"] == "synthetic-work-account"
            passed("exported " + source["agent"] + " entry starts its Bridge and calls the authenticated upstream fixture", {"sourceId":source["id"]})
        assert "retain OpenClaw comment" in openclaw_path.read_text(encoding="utf-8")
        assert "mode: 'merge'" in openclaw_path.read_text(encoding="utf-8")
        assert dsh_unrelated in dsh_path.read_text(encoding="utf-8")
        passed("JSON5 comments, provider fields and unrelated dynamic Cordis YAML survive shared writes without credential literals")

        # Native enable semantics are per-client; Claude owns its project preference toggles.
        togglable = [s for s in sources if s["agent"] not in {"claude", "workbuddy"}]
        harness.changes(office, [s["id"] for s in togglable], action="disable")
        disabled = snapshot()
        assert all(not b["enabled"] for b in disabled["bindings"] if b.get("connectionId") == office and b["agent"] not in {"claude", "workbuddy"})
        harness.changes(office, [s["id"] for s in togglable], action="enable")
        assert all(b["enabled"] for b in snapshot()["bindings"] if b.get("connectionId") == office)
        claude = next(s for s in sources if s["agent"] == "claude")
        assert "项目偏好" in harness.rpc("changes.preview", {"connectionId":office,"targetSourceIds":[claude["id"]],"action":"disable"}, error=True)
        passed("enable/disable follows each native format and leaves Claude's client-owned preferences untouched")

        harness.changes(local, [s["id"] for s in sources], mode="direct")
        for source in sources:
            client, _ = native_client(source, "local_notes", local)
            passed("exported " + source["agent"] + " direct configuration invokes the real stdio MCP fixture")

        kimi = next(s for s in sources if s["agent"] == "kimicode")
        _, removal = harness.changes(office, [kimi["id"]], action="remove")
        kimi_path = Path(kimi["path"])
        kimi_value = json.loads(kimi_path.read_text(encoding="utf-8"))
        kimi_value["unrelatedAfterRemoval"] = True
        kimi_path.write_text(json.dumps(kimi_value), encoding="utf-8")
        restored = harness.rpc("changes.restore", {"operationId":removal["results"][0]["operationId"]})
        assert restored["status"] == "restored"
        assert json.loads(kimi_path.read_text(encoding="utf-8"))["unrelatedAfterRemoval"]
        assert native_entry(kimi, "office_docs")["command"] == str(BINARY)
        passed("Kimi Code removal and restore preserve later unrelated edits and restore the original proxy entry")

        fresh = harness.rpc("connections.save", {"name":"later_notes","definition":{"command":sys.executable,"args":[str(FIXTURE),"--stdio"],"enabled":True}})["id"]
        conflict = harness.rpc("changes.preview", {"connectionId":fresh,"targetSourceIds":[openclaw["id"]]})
        openclaw_path.write_text(openclaw_path.read_text(encoding="utf-8") + "\n// concurrent edit\n", encoding="utf-8")
        assert harness.rpc("changes.apply", {"planId":conflict["planId"]})["results"][0]["status"] == "failed"
        passed("native JSON5 changes retain the common concurrent-file conflict protection")

        harness.changes(office, [dsh["id"]], action="remove")
        assert dsh_unrelated in dsh_path.read_text(encoding="utf-8")
        assert any(b["agent"] == "deepseek" and b.get("connectionId") == local for b in snapshot()["bindings"])
        passed("removing one DeepSeek MCP plugin preserves sibling MCP rows and unrelated tagged YAML")

        # Native Settings' MCP manager map is also manageable, without rewriting siblings.
        manager = "\n- id: mcp\n  name: '@deepseek-ai/dsh-mcp-manager'\n  config:\n    servers:\n      managed_docs:\n        transport: streamable-http\n        url: " + json.dumps(harness.url) + "\n        headers: {Authorization: synthetic-fixture-authorization}\n        enabled: true\n"
        dsh_path.write_text(dsh_path.read_text(encoding="utf-8") + manager, encoding="utf-8")
        managed = harness.rpc("connections.import", {"sourceId":dsh["id"],"name":"managed_docs"})["id"]
        assert harness.rpc("connections.probe", {"id":managed})["connection"]["status"] == "ready"
        harness.changes(managed, [dsh["id"]], action="disable")
        assert not next(b for b in snapshot()["bindings"] if b["name"] == "managed_docs")["enabled"]
        harness.changes(managed, [dsh["id"]], action="enable")
        _, removed_manager = harness.changes(managed, [dsh["id"]], action="remove")
        assert harness.rpc("changes.restore", {"operationId":removed_manager["results"][0]["operationId"]})["status"] == "restored"
        assert dsh_unrelated in dsh_path.read_text(encoding="utf-8")
        passed("DeepSeek native manager servers support import, enable/disable, removal and restore alongside plugin entries")

        malformed_dsh = record / "malformed-dsh.yml"
        for definition in [
            "- name: '@deepseek-ai/dsh-mcp-client'\n  disabled: 'false'\n  config: {serverName: bad, command: fixture}\n",
            "- name: '@deepseek-ai/dsh-mcp-manager'\n  config:\n    servers:\n      bad: {command: fixture, enabled: 'false'}\n",
        ]:
            malformed_dsh.write_text(definition, encoding="utf-8")
            assert "布尔值" in harness.rpc("sources.add", {"agent":"deepseek","path":str(malformed_dsh)}, error=True)
        passed("DeepSeek plugin and manager reject malformed enabled states instead of silently enabling a service")

        remote = harness.rpc("connections.save", {"name":"remote_native","definition":{"url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"},"enabled":True}})["id"]
        custom_sources = [s for s in sources if s["agent"] in {"zcode","deepseek","openclaw","kimicode"}]
        harness.changes(remote, [s["id"] for s in custom_sources], mode="direct")
        assert all(not b["changed"] and not b["error"] for b in snapshot()["bindings"] if b.get("connectionId") == remote)
        passed("new writable adapters round-trip native authenticated HTTP definitions with stable bindings")

        install_plan = harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":[kimi["id"]],"action":"disable"})
        before_uninstall = kimi_path.read_bytes()
        harness.launchers["kimicode"].unlink()
        missing = next(a for a in snapshot()["agents"] if a["id"] == "kimicode")
        assert missing["detected"] and not missing["installed"] and not missing["configurable"]
        assert "未安装" in harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":[kimi["id"]]}, error=True)
        blocked = harness.rpc("changes.apply", {"planId":install_plan["planId"]})
        assert blocked["results"][0]["status"] == "failed" and "未安装" in blocked["results"][0]["message"]
        assert kimi_path.read_bytes() == before_uninstall
        harness.install_clients()
        passed("uninstalling after preview disables the residual profile and blocks stale apply without writing the file")

        _, disabled_for_restore = harness.changes(local, [kimi["id"]], action="disable")
        before_restore = kimi_path.read_bytes()
        harness.launchers["kimicode"].unlink()
        assert "未安装" in harness.rpc("changes.restore", {"operationId":disabled_for_restore["results"][0]["operationId"]}, error=True)
        assert kimi_path.read_bytes() == before_restore
        harness.install_clients()
        assert harness.rpc("changes.restore", {"operationId":disabled_for_restore["results"][0]["operationId"]})["status"] == "restored"
        passed("restore is blocked for an uninstalled client and works after reinstalling and refreshing")

        invalid = {"transport_bad":{"command":"fixture","transport":4},
            "command_bad":{"url":harness.url,"command":4},
            "filters_bad":{"command":"fixture","enabledTools":["allowed",5]},
            "remote_environment":{"command":"fixture","environment_id":"ssh-production"},
            "milliseconds":{"command":"fixture","toolTimeoutMs":1501},
            "variable":{"url":harness.url,"bearerTokenEnvVar":"invalid-name"}}
        kimi_value = json.loads(kimi_path.read_text(encoding="utf-8"))
        kimi_value["mcpServers"].update(invalid)
        kimi_path.write_text(json.dumps(kimi_value), encoding="utf-8")
        bad = snapshot()
        assert all(next(b for b in bad["bindings"] if b["name"] == name)["error"] for name in invalid)
        assert "保留" in harness.rpc("changes.preview", {"connectionId":harness.rpc("connections.save", {"name":"__proto__","definition":{"command":"fixture"}})["id"],"targetSourceIds":[openclaw["id"]]}, error=True)
        passed("invalid native types, filters, environments, timeouts, credential references and reserved names fail explicitly")

        project = record / "project with spaces"
        project.mkdir()
        project_source = harness.rpc("sources.add", {"agent":"kimicode","scope":"project","project":str(project)})
        assert not Path(project_source["path"]).exists()
        assert "项目配置路径" in harness.rpc("sources.add", {"agent":"kimicode","scope":"project","project":str(project),"path":str(kimi_path)}, error=True)
        passed("installed Kimi project sources reject unrelated target paths without creating config files before apply")

        runtime_path = harness.data / "runtime.json"
        original_runtime = runtime_path.read_bytes()
        runtime = json.loads(original_runtime)
        requests = []

        class OldBackend(BaseHTTPRequestHandler):
            management_api = 8

            def do_GET(self):
                body = json.dumps({"application":"mcplink","instance":runtime["instance"],"managementApi":self.management_api}).encode()
                self.send_response(200)
                self.end_headers()
                self.wfile.write(body)

            def do_POST(self):
                requests.append(self.path)
                self.send_response(500)
                self.end_headers()

            def log_message(self, *_):
                pass

        old_backend = HTTPServer(("127.0.0.1", 0), OldBackend)
        thread = threading.Thread(target=old_backend.serve_forever, daemon=True)
        thread.start()
        try:
            runtime_path.write_text(json.dumps({**runtime,"port":old_backend.server_port}), encoding="utf-8")
            assert "旧后台" in harness.rpc("sources.add", {"agent":"kimicode"}, error=True)
            assert "旧后台" in harness.rpc("changes.preview", {"connectionId":office,"targetSourceIds":[kimi["id"]]}, error=True)
            assert "旧后台" in harness.rpc("sources.add", {"agent":"codex"}, error=True)
            assert "旧后台" in harness.rpc("changes.apply", {"planId":"stale"}, error=True)
            assert "旧后台" in harness.rpc("changes.restore", {"operationId":"stale"}, error=True)
            assert requests == [], "new native operations reached an incompatible backend"
            OldBackend.management_api = 13
            assert "旧后台" in harness.rpc("connections.delete", {"id":office,"keepNative":True}, error=True)
            assert requests == [], "keep-native was sent to a daemon that does not recognize it"
            harness.rpc("connections.delete", {"id":office,"keepNative":False}, error=True)
            assert requests == ["/rpc"], "ordinary deletion should still reach an API 13 daemon"
            requests.clear()
            OldBackend.management_api = 14
            assert "旧后台" in harness.rpc("snapshot", error=True)
            assert "旧后台" in harness.rpc("sources.add", {"agent":"workbuddy"}, error=True)
            assert "旧后台" in harness.rpc("connections.export_config", {"targetAgent":"qwenwork","connectionIds":[office],"mode":"direct"}, error=True)
            assert "旧后台" in harness.rpc("changes.preview", {"connectionId":office,"targetSourceIds":["native:workbuddy:fixture"]}, error=True)
            assert requests == [], "office integration operations reached an API 14 daemon"
            OldBackend.management_api = 15
            for method, params in [
                ("snapshot", {}),
                ("sources.add", {"agent":"qwenwork"}),
                ("agents.launcher", {"agent":"qwenwork","path":"fixture"}),
                ("connections.import", {"sourceId":"native:qwenwork:fixture","name":"fixture"}),
                ("changes.preview", {"connectionId":office,"targetSourceIds":["native:qwenwork:fixture"]}),
                ("changes.apply", {"planId":"stale"}),
                ("changes.restore", {"operationId":"stale"}),
            ]:
                assert "旧后台" in harness.rpc(method, params, error=True)
            assert requests == [], "Qwen native operations reached an API 15 daemon"
            OldBackend.management_api = 16
            for method, params in [
                ("snapshot", {}),
                ("sources.add", {"agent":"pi"}),
                ("sources.add", {"agent":"workbuddy"}),
                ("agents.launcher", {"agent":"pi","path":"fixture"}),
                ("connections.import", {"sourceId":"native:pi:fixture","name":"fixture"}),
                ("connections.import", {"sourceId":"native:workbuddy:fixture","name":"fixture"}),
                ("changes.preview", {"connectionId":office,"targetSourceIds":["native:pi:fixture"]}),
                ("changes.preview", {"connectionId":office,"targetSourceIds":["native:workbuddy:fixture"]}),
                ("changes.apply", {"planId":"stale"}),
                ("changes.restore", {"operationId":"stale"}),
            ]:
                assert "旧后台" in harness.rpc(method, params, error=True)
            assert requests == [], "Pi native operations reached an API 16 daemon"
        finally:
            runtime_path.write_bytes(original_runtime)
            old_backend.shutdown()
            old_backend.server_close()
            thread.join(timeout=5)
        passed("API 8 backends reject client configuration operations before bypassing installation-state checks")
        passed("API 13 blocks keep-native before dispatch and accepts ordinary deletion requests")
        passed("API 14 blocks office inventory, WorkBuddy configuration and manual exports before dispatch")
        passed("API 15 blocks Qwen native inventory, writes and recovery before dispatch")
        passed("API 16 blocks Pi and current WorkBuddy native operations before dispatch")

        assert retired.read_text(encoding="utf-8").startswith('{"servers"')
        assert old_kimi.read_text(encoding="utf-8").startswith('{"mcpServers"')
        passed("support-list changes do not uninstall clients or delete their native configuration files")
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        print("FAIL", error, flush=True)
        traceback.print_exc()
    finally:
        harness.close(False)
    report = {"conclusion":conclusion,"error":error,"scenarios":results,
        "executed_at":datetime.now(timezone.utc).isoformat(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        "script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
        "scope":"isolated native formats, independent TOML/JSON5/YAML readers, actual exported commands and MCP SDK; no installed vendor model/client execution claim"}
    (record / "report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n",encoding="utf-8")
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
