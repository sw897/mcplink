"""Validate WorkBuddy/QwenWork native transactions and offline office-client imports."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
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
        assert len(initial["agents"]) == 12
        assert not any(a["installed"] for a in initial["agents"])
        assert {a["id"] for a in initial["agents"] if a["configurable"]} == {"doubaowork"}
        assert {a["id"] for a in initial["agents"] if a["supportsImport"]} == {"qwenwork", "doubaowork"}
        assert not any(s["agent"] == "doubaowork" for s in initial["sources"])
        assert "界面导入" in harness.rpc("sources.add", {"agent":"doubaowork"}, error=True)
        assert "未安装" in harness.rpc("sources.add", {"agent":"qwenwork"}, error=True)
        source = next(s for s in initial["sources"] if s["agent"] == "workbuddy")
        path = Path(source["path"])
        assert path == harness.home / ".workbuddy/mcp.json" and not path.exists()
        assert "未安装" in harness.rpc("sources.add", {"agent":"workbuddy"}, error=True)
        passed("inventory distinguishes installation, native write access and offline connector imports")

        # Real distribution names, represented by presence-only fixtures. No
        # vendor client is launched by this isolated inventory acceptance test.
        def office_installed(agent):
            item = next(a for a in snapshot()["agents"] if a["id"] == agent)
            assert item["supportsImport"]
            if agent == "doubaowork":
                assert item["management"] == "manual" and item["configurable"]
                assert item["profilePath"] is None
            else:
                assert item["management"] == "writable" and item["configurable"] == item["installed"]
                assert Path(item["profilePath"]) == harness.home / ".qwenworkcn"
            return item["installed"]

        if sys.platform == "darwin":
            import plistlib

            def office_bundle(name, executable):
                bundle = harness.home / "Applications" / name
                program = bundle / "Contents/MacOS" / executable
                program.parent.mkdir(parents=True)
                (bundle / "Contents/Info.plist").write_bytes(plistlib.dumps({"CFBundleExecutable":executable}))
                return bundle, program

            for name in ("Doubao.app", "豆包.app"):
                _, program = office_bundle(name, "Doubao")
                program.touch()
            assert not office_installed("doubaowork")
            for agent, name, executable in (("qwenwork", "QwenWorkCN.app", "QwenWorkCN"),
                                             ("doubaowork", "DoubaoWork.app", "DoubaoWork")):
                bundle, program = office_bundle(name, executable)
                assert not office_installed(agent)
                program.touch()
                assert office_installed(agent)
                program.unlink()
                assert not office_installed(agent)
                shutil.rmtree(bundle)
            for agent, name, executable in (("qwenwork", "QwenWork.app", "QwenWorkCN"),
                                             ("qwenwork", "千问办公.app", "QwenWorkCN"),
                                             ("doubaowork", "豆包工作.app", "DoubaoWork")):
                bundle, program = office_bundle(name, executable)
                program.touch()
                assert office_installed(agent)
                shutil.rmtree(bundle)
        elif os.name == "nt":
            local_apps = harness.home / "AppData/Local"
            consumer = local_apps / "Doubao/Application/Doubao.exe"
            consumer.parent.mkdir(parents=True)
            consumer.touch()
            other_consumer = local_apps / "Programs/Doubao/Doubao.exe"
            other_consumer.parent.mkdir(parents=True)
            other_consumer.touch()
            assert not office_installed("doubaowork")
            for agent, relative in (("qwenwork", "Programs/QwenWorkCN/QwenWorkCN.exe"),
                                     ("doubaowork", "Programs/DoubaoWork/DoubaoWork.exe"),
                                     ("doubaowork", "DoubaoWork/Application/DoubaoWork.exe")):
                program = local_apps / relative
                program.parent.mkdir(parents=True)
                assert not office_installed(agent)
                program.touch()
                assert office_installed(agent)
                program.unlink()
                assert not office_installed(agent)
        if sys.platform == "darwin" or os.name == "nt":
            assert not any(s["agent"] == "doubaowork" for s in snapshot()["sources"])
            passed("official office program names and aliases are detected; consumer Doubao, broken bundles and removed programs stay uninstalled")

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

        qwen = next(s for s in snapshot()["sources"] if s["agent"] == "qwenwork")
        qwen_path = Path(qwen["path"])
        assert qwen_path == harness.home / ".qwenworkcn/mcp.json"
        qwen_path.parent.mkdir(parents=True)
        qwen_original = {"keepPreference":{"value":"保留"},"cliConnectors":{"keep_cli":{"command":"never-run"}},
            "mcpServers":{"qwen_docs":{"type":"streamable-http","url":harness.url,
                "headers":{"Authorization":"synthetic-fixture-authorization"},"enabled":True}}}
        qwen_path.write_text(json.dumps(qwen_original, ensure_ascii=False, indent=2), encoding="utf-8")
        qwen_id = harness.rpc("connections.import", {"sourceId":qwen["id"],"name":"qwen_docs"})["id"]
        assert harness.rpc("connections.probe", {"id":qwen_id})["connection"]["status"] == "ready"
        before_qwen = qwen_path.read_bytes()
        plan = harness.rpc("changes.preview", {"connectionId":qwen_id,"targetSourceIds":[qwen["id"]],"mode":"shared"})
        assert qwen_path.read_bytes() == before_qwen
        assert harness.rpc("changes.apply", {"planId":plan["planId"]})["results"][0]["status"] == "applied"
        value = json.loads(qwen_path.read_text(encoding="utf-8"))
        assert value["keepPreference"] == qwen_original["keepPreference"]
        assert value["cliConnectors"] == qwen_original["cliConnectors"]
        entry = value["mcpServers"]["qwen_docs"]
        assert entry["type"] == "stdio" and entry["enabled"] is True
        assert "synthetic-fixture-authorization" not in json.dumps(value)
        client = McpClient(qwen_id, harness.environment, "qwen-native-shared", entry)
        harness.clients.append(client)
        assert "result" in client.call("tools/call", {"name":"read_document","arguments":{"id":"weekly-fixture"}})
        harness.changes(qwen_id, [qwen["id"]], action="disable")
        assert json.loads(qwen_path.read_text(encoding="utf-8"))["mcpServers"]["qwen_docs"]["enabled"] is False
        harness.changes(qwen_id, [qwen["id"]], action="enable")
        _, removed = harness.changes(qwen_id, [qwen["id"]], action="remove")
        value = json.loads(qwen_path.read_text(encoding="utf-8"));value["laterPreference"] = True
        qwen_path.write_text(json.dumps(value), encoding="utf-8")
        assert harness.rpc("changes.restore", {"operationId":removed["results"][0]["operationId"]})["status"] == "restored"
        assert json.loads(qwen_path.read_text(encoding="utf-8"))["laterPreference"] is True
        passed("QwenWork strict native JSON supports shared MCP calls, enable/disable and recovery while preserving CLI connectors and preferences")

        qwen_valid = qwen_path.read_text(encoding="utf-8")
        value = json.loads(qwen_valid);value["cliConnectors"]["local_notes"] = {"command":"never-run"}
        qwen_path.write_text(json.dumps(value), encoding="utf-8")
        assert "同名 CLI" in harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":[qwen["id"]]}, error=True)
        value["cliConnectors"]["qwen_docs"] = {"command":"never-run"}
        qwen_path.write_text(json.dumps(value), encoding="utf-8")
        assert "同名" in harness.rpc("connections.import", {"sourceId":qwen["id"],"name":"qwen_docs"}, error=True)
        for content in ['// JSONC is not a QwenWork document\n' + qwen_valid,
                json.dumps({"mcpServers":{"bad":{"type":"sse","url":harness.url}}}),
                json.dumps({"mcpServers":{"bad":{"command":"fixture","cwd":str(record)}}}),
                json.dumps({"mcpServers":{"bad":{"command":"fixture","timeout":1000}}})]:
            qwen_path.write_text(content, encoding="utf-8")
            harness.rpc("connections.import", {"sourceId":qwen["id"],"name":"bad"}, error=True)
            assert qwen_path.read_text(encoding="utf-8") == content
        qwen_path.write_text(qwen_valid, encoding="utf-8")
        assert "仅适配当前用户" in harness.rpc("sources.add", {"agent":"qwenwork","path":str(record/"sdk-config.json")}, error=True)
        passed("QwenWork rejects JSONC, cross-bucket collisions, SDK paths and unverified execution fields without changing the native document")

        if sys.platform == "darwin":
            qwen_profile = harness.home / "Library/Application Support/QwenWorkCN"
        elif os.name == "nt":
            qwen_profile = harness.home / "AppData/Roaming/QwenWorkCN"
        else:
            qwen_profile = harness.home / ".config/QwenWorkCN"
        for marker in ["data/agents.identity-v2.db", "data/.local-identity-format.json", "identity-transition/v2/state.json"]:
            pending = harness.rpc("changes.preview", {"connectionId":local,"targetSourceIds":[qwen["id"]]})
            marker_path = qwen_profile / marker
            marker_path.parent.mkdir(parents=True, exist_ok=True);marker_path.touch()
            before_guard = qwen_path.read_bytes()
            private = snapshot()
            item = next(a for a in private["agents"] if a["id"] == "qwenwork")
            assert item["management"] == "manual" and item["supportsImport"] and item["configurable"]
            assert item["profilePath"] is None and "身份隔离" in item["managementReason"]
            assert next(s for s in private["sources"] if s["id"] == qwen["id"])["readOnly"]
            assert harness.rpc("changes.apply", {"planId":pending["planId"]})["results"][0]["status"] == "failed"
            assert qwen_path.read_bytes() == before_guard
            before_export = native_state()
            assert export("qwenwork", [remote])["requiresImport"]
            assert native_state() == before_export
            marker_path.unlink()
            if marker.startswith("identity-transition"):
                shutil.rmtree(qwen_profile / "identity-transition")
        db_path = qwen_profile / "data/agents.db"
        with sqlite3.connect(db_path) as db:
            db.execute("CREATE TABLE identity_connector_heads(identity_key TEXT PRIMARY KEY)")
            db.execute("INSERT INTO identity_connector_heads VALUES ('synthetic-identity')")
        db.close()
        assert next(a for a in snapshot()["agents"] if a["id"] == "qwenwork")["management"] == "manual"
        db_path.unlink()
        assert next(a for a in snapshot()["agents"] if a["id"] == "qwenwork")["management"] == "writable"
        _, protected_removal = harness.changes(qwen_id, [qwen["id"]], action="remove")
        marker_path = qwen_profile / "data/agents.identity-v2.db";marker_path.touch()
        before_guard = qwen_path.read_bytes()
        harness.rpc("changes.restore", {"operationId":protected_removal["results"][0]["operationId"]}, error=True)
        assert qwen_path.read_bytes() == before_guard
        marker_path.unlink()
        assert harness.rpc("changes.restore", {"operationId":protected_removal["results"][0]["operationId"]})["status"] == "restored"
        passed("QwenWork identity markers or populated identity tables select manual import and block stale apply and restore without writing legacy JSON")

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
        original = '{"theme":"light","keepOfficePreference":true,"mcpServers":{"work_remote":' + json.dumps({
            "type":"streamableHttp","url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"},
            "timeout":6000}) + '}}\n'
        path.write_text(original, encoding="utf-8")
        native = harness.rpc("connections.import", {"sourceId":source["id"],"name":"work_remote"})["id"]
        detail = harness.rpc("connections.detail", {"id":native})["definition"]
        assert detail["startupTimeout"] == 6 and detail["timeout"] == 6
        assert detail["excludeTools"] == []
        assert harness.rpc("connections.probe", {"id":native})["connection"]["status"] == "ready"
        plan = harness.rpc("changes.preview", {"connectionId":native,"targetSourceIds":[source["id"]]})
        assert path.read_text(encoding="utf-8") == original
        assert harness.rpc("changes.apply", {"planId":plan["planId"]})["results"][0]["status"] == "applied"
        assert json.loads(path.read_text(encoding="utf-8"))["keepOfficePreference"] is True
        assert next(b for b in snapshot()["bindings"] if b.get("connectionId") == native)["changed"] is False
        assert "原客户端" in harness.rpc("changes.preview", {"connectionId":native,"targetSourceIds":[source["id"]],"action":"disable"}, error=True)
        _, removal = harness.changes(native, [source["id"]], action="remove")
        later = json.loads(path.read_text(encoding="utf-8"));later["laterPreference"] = True
        path.write_text(json.dumps(later), encoding="utf-8")
        assert harness.rpc("changes.restore", {"operationId":removal["results"][0]["operationId"]})["status"] == "restored"
        assert json.loads(path.read_text(encoding="utf-8"))["laterPreference"] is True
        passed("WorkBuddy strict JSON, native HTTP and uniform startup/request timeouts round-trip with recovery")

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
                {"command":"fixture","runtime":{"type":"node"}}, {"command":"fixture","env":{"KEY":"${VAR}"}},
                {"url":harness.url,"disabledTools":["search_documents"]}):
            content = json.dumps({"mcpServers":{"bad":definition}})
            bad_path.write_text(content, encoding="utf-8")
            harness.rpc("connections.import", {"sourceId":bad_source["id"],"name":"bad"}, error=True)
            assert bad_path.read_text(encoding="utf-8") == content
        content = '// JSONC is not a WorkBuddy user document\n{"mcpServers":{}}'
        bad_path.write_text(content, encoding="utf-8")
        harness.rpc("connections.import", {"sourceId":bad_source["id"],"name":"bad"}, error=True)
        assert bad_path.read_text(encoding="utf-8") == content
        bad_path.write_text(json.dumps({"mcpServers":{"legacy_seconds":{"url":harness.url,"timeout":30}}}), encoding="utf-8")
        seconds = harness.rpc("connections.import", {"sourceId":bad_source["id"],"name":"legacy_seconds"})["id"]
        detail = harness.rpc("connections.detail", {"id":seconds})["definition"]
        assert detail["startupTimeout"] == detail["timeout"] == 30
        limited = harness.rpc("connections.save", {"name":"workbuddy_limited","definition":{
            "url":harness.url,"headers":{"Authorization":"synthetic-fixture-authorization"},
            "excludeTools":["search_documents"]}})["id"]
        assert "共享" in harness.rpc("changes.preview", {"connectionId":limited,"targetSourceIds":[project_source["id"]]}, error=True)
        harness.rpc("connections.probe", {"id":limited})
        harness.changes(limited, [project_source["id"]], mode="shared")
        entry = json.loads(project_path.read_text())["mcpServers"]["workbuddy_limited"]
        assert "disabledTools" not in entry
        limited_client = McpClient(limited, harness.environment, "workbuddy-filtered-shared", entry)
        harness.clients.append(limited_client)
        assert [t["name"] for t in limited_client.call("tools/list")["result"]["tools"]] == ["read_document"]
        passed("WorkBuddy rejects JSONC and ignored file filters; its legacy seconds and shared filters retain actual semantics")

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
