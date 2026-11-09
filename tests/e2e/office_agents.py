"""Validate WorkBuddy native transactions and offline QwenWork/DoubaoWork imports."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import sqlite3
import shutil
import subprocess
import sys
import traceback

from run import BINARY, FIXTURE, Harness, McpClient


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = Harness(record, installed_agents=False)
    result, error = "PASS", None
    checks = []

    def passed(name):
        checks.append(name)
        print("PASS", name, flush=True)

    def export(agent, ids, mode="shared", fail=False):
        args = [str(BINARY), "connections", "export-config", "--agent", agent, "--mode", mode]
        for connection in ids:
            args += ["--connection", connection]
        proc = subprocess.run(args, env=harness.environment, text=True, capture_output=True, timeout=60)
        value = json.loads(proc.stdout)
        if fail:
            assert proc.returncode == 1 and "error" in value, value
            return value["error"]["message"]
        assert proc.returncode == 0 and "ok" in value, value
        assert value["ok"]["requiresImport"] is True
        return value["ok"]

    def snapshot():
        return harness.rpc("snapshot")

    def native_state():
        value = snapshot()
        return value["bindings"], value["history"], value["sources"]

    try:
        initial = snapshot()
        assert len(initial["agents"]) == 11
        assert not any(a["installed"] for a in initial["agents"])
        assert {a["id"] for a in initial["agents"] if a["configurable"]} == {"qwenwork", "doubaowork"}
        assert not any(s["agent"] in {"qwenwork", "doubaowork"} for s in initial["sources"])
        for agent in ("qwenwork", "doubaowork"):
            assert "界面导入" in harness.rpc("sources.add", {"agent":agent}, error=True)
        source = next(s for s in initial["sources"] if s["agent"] == "workbuddy")
        path = Path(source["path"])
        assert path == harness.home / ".workbuddy/mcp.json" and not path.exists()
        assert "未安装" in harness.rpc("sources.add", {"agent":"workbuddy"}, error=True)
        passed("inventory distinguishes installation, native write access and offline connector imports")

        local = harness.rpc("connections.import", {"sourceId":"codex:user", "name":"local_notes"})["id"]
        remote = harness.rpc("connections.import", {"sourceId":"hermes:user", "name":"office_docs"})["id"]
        before = native_state()
        for agent in ("qwenwork", "doubaowork"):
            value = export(agent, [remote], "direct")
            entry = value["entries"][0]["definition"]
            assert entry["url"] == harness.url
            assert entry["headers"] == {"Authorization":"synthetic-fixture-authorization"}
            assert entry["type"] == ("streamable-http" if agent == "qwenwork" else "http")
            if agent == "qwenwork":
                assert value["config"] == {"mcpServers":{"office_docs":entry}}
            else:
                assert value["config"] is None
            assert "检查共享连接" in export(agent, [local], fail=True)
        assert native_state() == before
        passed("named CLI exports exact HTTP configuration without a client install, native writes or fictional bindings")

        edge = {"command":sys.executable,"args":["a path/含空格.py",'literal "quotes"',"", "a\\b"],"env":{"FIXTURE_VALUE":"value with spaces\nnext"}}
        edge_id = harness.rpc("connections.save", {"name":"edge_args","definition":edge})["id"]
        for agent in ("qwenwork", "doubaowork"):
            exported = export(agent, [edge_id], "direct")["entries"][0]["definition"]
            assert exported == {"type":"stdio", **edge}
        batch = export("qwenwork", [local, remote], "direct")
        assert set(batch["config"]["mcpServers"]) == {"local_notes", "office_docs"}
        assert "重复" in export("qwenwork", [local, local], "direct", fail=True)
        same_name = harness.rpc("connections.save", {"name":"local_notes","definition":{"command":sys.executable}})["id"]
        assert "重复" in export("doubaowork", [local, same_name], "direct", fail=True)
        for params in ({"targetAgent":"codex","mode":"direct","connectionIds":[local]},
                       {"targetAgent":"qwenwork","mode":"invalid","connectionIds":[local]},
                       {"targetAgent":"qwenwork","mode":"direct","connectionIds":[]}):
            harness.rpc("connections.export_config", params, error=True)
        passed("batch export preserves argument boundaries, Unicode and environment values and rejects collisions or invalid requests")

        assert harness.rpc("connections.probe", {"id":remote})["connection"]["status"] == "ready"
        assert harness.rpc("connections.probe", {"id":local})["connection"]["status"] == "ready"
        before = native_state()
        for agent in ("qwenwork", "doubaowork"):
            value = export(agent, [remote])
            entry = value["entries"][0]["definition"]
            assert entry["type"] == "stdio" and entry["command"] == str(BINARY)
            assert entry["args"] == ["bridge", "--connection", remote]
            assert "synthetic-fixture-authorization" not in json.dumps(value)
            client = McpClient(remote, harness.environment, agent, entry)
            harness.clients.append(client)
            assert len(client.call("tools/list")["result"]["tools"]) == 2
            output = client.call("tools/call", {"name":"read_document","arguments":{"id":"weekly-fixture"}})
            assert json.loads(output["result"]["content"][0]["text"])["account"] == "synthetic-work-account"
        assert native_state() == before
        harness.rpc("gateway.pause")
        assert export("qwenwork", [remote])["gatewayPaused"] is True
        harness.rpc("gateway.resume")
        passed("both exported STDIO bridges perform real authenticated MCP calls while credentials remain in mcplink")

        for index, option in enumerate(({"cwd":str(record)}, {"includeTools":["read_document"]},
                {"timeout":30}, {"startupTimeout":4}, {"resourcesDisabled":True},
                {"enabled":False})):
            connection = harness.rpc("connections.save", {"name":f"restricted_{index}","definition":{
                "command":sys.executable,"args":[str(FIXTURE),"--stdio"], **option}})["id"]
            for agent in ("qwenwork", "doubaowork"):
                assert "共享模式" in export(agent, [connection], "direct", fail=True)
        harness.rpc("credentials.disconnect", {"id":edge_id})
        assert "断开" in export("qwenwork", [edge_id], "direct", fail=True)
        passed("unsupported execution limits and disconnected connections fail explicitly instead of losing semantics")

        harness.install_clients()
        custom = record / "custom-apps"
        custom.mkdir()
        if sys.platform == "darwin":
            bundle = harness.home / "Applications/WorkBuddy.app"
            launcher = Path(shutil.move(str(bundle), str(custom / "WorkBuddy.app")))
        else:
            launcher = Path(shutil.move(str(harness.launchers["workbuddy"]), str(custom / "WorkBuddy.exe")))
        assert not next(a for a in snapshot()["agents"] if a["id"] == "workbuddy")["installed"]
        harness.rpc("agents.launcher", {"agent":"workbuddy","path":str(launcher)})
        assert next(a for a in snapshot()["agents"] if a["id"] == "workbuddy")["installed"]
        passed("a verified WorkBuddy desktop program at a custom location enables native configuration")
        path.parent.mkdir(parents=True)
        original = '// keep office preference\n{"theme":"light","mcpServers":{"work_remote":' + json.dumps({
            "type":"streamableHttp","url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"},
            "timeout":6000,"disabledTools":["search_documents"]}) + '}}\n'
        path.write_text(original, encoding="utf-8")
        native = harness.rpc("connections.import", {"sourceId":source["id"],"name":"work_remote"})["id"]
        detail = harness.rpc("connections.detail", {"id":native})["definition"]
        assert detail["startupTimeout"] == 6 and detail["timeout"] is None
        assert detail["excludeTools"] == ["search_documents"]
        assert harness.rpc("connections.probe", {"id":native})["connection"]["status"] == "ready"
        plan = harness.rpc("changes.preview", {"connectionId":native,"targetSourceIds":[source["id"]]})
        assert path.read_text(encoding="utf-8") == original
        assert harness.rpc("changes.apply", {"planId":plan["planId"]})["results"][0]["status"] == "applied"
        assert "// keep office preference" in path.read_text(encoding="utf-8")
        assert next(b for b in snapshot()["bindings"] if b.get("connectionId") == native)["changed"] is False
        assert "原客户端" in harness.rpc("changes.preview", {"connectionId":native,"targetSourceIds":[source["id"]],"action":"disable"}, error=True)
        _, removal = harness.changes(native, [source["id"]], action="remove")
        path.write_text(path.read_text(encoding="utf-8") + "\n// unrelated later edit\n", encoding="utf-8")
        assert harness.rpc("changes.restore", {"operationId":removal["results"][0]["operationId"]})["status"] == "restored"
        assert "// unrelated later edit" in path.read_text(encoding="utf-8")
        passed("WorkBuddy HTTP, millisecond connection timeout and tool blacklist round-trip with native transaction recovery")

        project = record / "project with spaces"
        project.mkdir()
        project_source = harness.rpc("sources.add", {"agent":"workbuddy","scope":"project","project":str(project)})
        project_path = Path(project_source["path"])
        assert project_path == project / ".workbuddy/mcp.json" and not project_path.exists()
        assert "项目配置路径" in harness.rpc("sources.add", {"agent":"workbuddy","scope":"project","project":str(project),"path":str(path)}, error=True)
        harness.changes(remote, [project_source["id"]], mode="shared")
        entry = json.loads(project_path.read_text(encoding="utf-8"))["mcpServers"]["office_docs"]
        assert entry["type"] == "stdio" and "enabled" not in entry
        client = McpClient(remote, harness.environment, "workbuddy-project", entry)
        harness.clients.append(client)
        assert "result" in client.call("tools/call", {"name":"read_document","arguments":{"id":"weekly-fixture"}})
        conflict = harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":[project_source["id"]]})
        project_path.write_text(project_path.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        assert harness.rpc("changes.apply", {"planId":conflict["planId"]})["results"][0]["status"] == "failed"
        passed("explicit WorkBuddy project sources use the documented path and retain Bridge and conflict protections")

        bad_path = record / "workbuddy-invalid.json"
        bad_source = harness.rpc("sources.add", {"agent":"workbuddy","path":str(bad_path)})
        for definition in ({"type":"sse","url":harness.url}, {"command":"fixture","timeout":1501},
                {"command":"fixture","runtime":{"type":"node"}}, {"command":"fixture","env":{"KEY":"${VAR}"}}):
            content = json.dumps({"mcpServers":{"bad":definition}})
            bad_path.write_text(content, encoding="utf-8")
            harness.rpc("connections.import", {"sourceId":bad_source["id"],"name":"bad"}, error=True)
            assert bad_path.read_text(encoding="utf-8") == content
        passed("unsupported WorkBuddy transports, subsecond timeouts, runtime declarations and credential templates stay untouched")

        codex_path = harness.home / ".codex/config.toml"
        codex_path.write_text(codex_path.read_text(encoding="utf-8") + '\n[mcp_servers.native_options]\ncommand = "fixture"\nfuture_native_option = true\n', encoding="utf-8")
        unknown = harness.rpc("connections.import", {"sourceId":"codex:user","name":"native_options"})["id"]
        for agent in ("qwenwork", "doubaowork"):
            assert "专属选项" in export(agent, [unknown], "direct", fail=True)
        assert "专属选项" in harness.rpc("changes.preview", {"connectionId":unknown,"targetSourceIds":[project_source["id"]]}, error=True)
        filter_path = record / "hermes-filter.yaml"
        filter_path.write_text(json.dumps({"mcp_servers":{"glob_filter":{"command":sys.executable,
            "args":[str(FIXTURE),"--stdio"],"tools":{"exclude":["search_*"]}}}}), encoding="utf-8")
        filter_source = harness.rpc("sources.add", {"agent":"hermes","path":str(filter_path)})
        filtered = harness.rpc("connections.import", {"sourceId":filter_source["id"],"name":"glob_filter"})["id"]
        assert "通配符" in harness.rpc("changes.preview", {"connectionId":filtered,"targetSourceIds":[project_source["id"]]}, error=True)
        passed("office direct adapters refuse unknown source options and incompatible Hermes wildcard filters")

        # Previously retired WorkBuddy records keep their IDs, permissions and binding ownership.
        with sqlite3.connect(harness.data / "mcplink.sqlite3") as db:
            db.execute("INSERT INTO sources(id,document) VALUES(?,?)", ("workbuddy:legacy", json.dumps({
                "id":"workbuddy:legacy","agent":"workbuddy","label":"legacy read-only","path":str(path),
                "project":None,"readOnly":True,"scope":"user"})))
        restored_sources = [s for s in snapshot()["sources"] if s["agent"] == "workbuddy" and Path(s["path"]) == path]
        assert len(restored_sources) == 1 and restored_sources[0]["readOnly"]
        assert restored_sources[0]["id"] == "workbuddy:legacy"
        assert "只读" in harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":["workbuddy:legacy"]}, error=True)
        passed("restored historical WorkBuddy sources keep their IDs and read-only boundary")
    except Exception as exception:
        result, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(False)
        report = {"conclusion":result,"error":error,"checks":checks,
            "executed_at":datetime.now(timezone.utc).isoformat(),
            "binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),
            "scope":"isolated CLI, native file transactions and real MCP Bridge calls; vendor desktop loading not executed"}
        (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    raise SystemExit(result != "PASS")


if __name__ == "__main__":
    main()
