"""Exercise Pi built-in MCP through the mcplink CLI and, optionally, the published Pi runtime."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sys
import traceback

from run import BINARY, FIXTURE, ROOT, Harness, McpClient


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--pi-package", type=Path)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = Harness(record, installed_agents=False)
    checks = []
    native = {"conclusion": "NOT_RUN", "reason": "Pass --pi-package for the official Pi CLI/runtime"}
    conclusion, error = "PASS", None

    def passed(name):
        checks.append(name)
        print("PASS", name, flush=True)

    def snapshot():
        return harness.rpc("snapshot")

    def cli(*arguments, params=None, fail=False):
        proc = subprocess.run([str(BINARY), *arguments], input=json.dumps(params) if params is not None else None,
                              env=harness.environment, capture_output=True, text=True, timeout=60)
        value = json.loads(proc.stdout)
        assert proc.returncode == (1 if fail else 0), (arguments, value)
        return value["error"] if fail else value["ok"]

    try:
        initial = snapshot()
        item = next(a for a in initial["agents"] if a["id"] == "pi")
        assert item["label"] == "Pi Agent" and not item["installed"] and not item["configurable"]
        assert item["management"] == "writable" and not item["supportsImport"]
        source = next(s for s in initial["sources"] if s["agent"] == "pi")
        path = Path(source["path"])
        assert path == harness.home / ".pi/agent/mcp.json" and not path.exists()
        assert "未安装" in cli("sources", "add", "--agent", "pi", fail=True)["message"]
        assert not path.exists()
        passed("uninstalled Pi stays visible and cannot create a native file")

        harness.install_clients()
        added = cli("sources", "add", "--agent", "pi")
        assert added["agent"] == "pi" and Path(added["path"]) == path and not path.exists()
        assert cli("sources", "add", "--agent", "qwenwork")["agent"] == "qwenwork"
        passed("named source commands expose Pi and the existing Qwen native adapter without writing files")

        path.parent.mkdir(parents=True)
        original = {"autoEnableCodemode": False, "keepPreference": {"value": "保留"}, "mcpServers": {
            "pi_docs": {"type": "http", "url": harness.url,
                        "headers": {"Authorization": "synthetic-fixture-authorization"},
                        "description": "合成文档", "exposure": "direct"}}}
        path.write_text("\ufeff" + json.dumps(original, ensure_ascii=False, indent=2), encoding="utf-8")
        connection = cli("connections", "import", "--source", source["id"], "--name", "pi_docs")["id"]
        assert cli("connections", "probe", connection)["connection"]["status"] == "ready"
        before = path.read_bytes()
        plan = cli("changes", "preview", "--connection", connection, "--target", source["id"], "--mode", "shared")
        assert path.read_bytes() == before
        assert cli("changes", "apply", plan["planId"])["results"][0]["status"] == "applied"
        value = json.loads(path.read_text(encoding="utf-8-sig"))
        entry = value["mcpServers"]["pi_docs"]
        assert path.read_bytes().startswith(b"\xef\xbb\xbf")
        assert value["keepPreference"] == original["keepPreference"] and value["autoEnableCodemode"] is False
        assert entry["exposure"] == "direct" and entry["description"] == "合成文档"
        assert "synthetic-fixture-authorization" not in path.read_text(encoding="utf-8")
        client = McpClient(connection, harness.environment, "pi-native-shared", entry)
        harness.clients.append(client)
        assert "result" in client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
        passed("strict JSON with a BOM preserves client preferences and exposure while its Bridge calls the upstream")

        harness.changes(connection, [source["id"]], action="disable")
        assert json.loads(path.read_text(encoding="utf-8-sig"))["mcpServers"]["pi_docs"]["enabled"] is False
        harness.changes(connection, [source["id"]], action="enable")
        _, removal = harness.changes(connection, [source["id"]], action="remove")
        value = json.loads(path.read_text(encoding="utf-8-sig"));value["laterPreference"] = True
        path.write_text(json.dumps(value), encoding="utf-8")
        assert harness.rpc("changes.restore", {"operationId": removal["results"][0]["operationId"]})["status"] == "restored"
        assert json.loads(path.read_text())["laterPreference"] is True
        passed("enable, disable, removal and recovery preserve later unrelated edits and native presentation")

        valid = path.read_text()
        sentinel = record / "dynamic-command-must-not-run"
        invalid = [
            {"url": harness.url, "enabled": "false"},
            {"url": harness.url, "command": 1},
            {"command": sys.executable, "cwd": 1},
            {"type": "sse", "url": harness.url},
            {"url": harness.url, "timeout": 1.5},
            {"url": harness.url, "auth": {"provider": "must-not-read-native-credentials"}},
            {"url": harness.url, "oauth": {"clientId": "synthetic"}},
            {"url": harness.url, "exposure": "hidden"},
            {"url": harness.url, "toolExposure": {"*": "hidden"}},
            {"url": harness.url, "headers": {"Authorization": "!touch " + str(sentinel)}},
            {"url": harness.url, "headers": {"Authorization": "${MISSING_PI_NATIVE_SECRET}"}},
            {"command": sys.executable, "cwd": "."},
            {"command": "~/bin/not-a-real-server"},
            {"enabled": False},
        ]
        for entry in invalid:
            text = json.dumps({"mcpServers": {"bad": entry}})
            path.write_text(text, encoding="utf-8")
            cli("connections", "import", "--source", source["id"], "--name", "bad", fail=True)
            assert path.read_text() == text and not sentinel.exists()
        path.write_text("// JSONC is not Pi built-in MCP\n" + valid, encoding="utf-8")
        cli("connections", "import", "--source", source["id"], "--name", "pi_docs", fail=True)
        path.write_text(valid, encoding="utf-8")
        passed("JSONC, unsupported transports, dynamic credentials, OAuth, timeouts and visibility restrictions stay untouched")

        collision = json.loads(valid)
        collision["mcpServers"]["pi-docs"] = {"url": harness.url}
        path.write_text(json.dumps(collision), encoding="utf-8")
        assert "冲突" in harness.rpc("changes.preview", {"connectionId": connection, "targetSourceIds": [source["id"]]}, error=True)
        path.write_text(valid, encoding="utf-8")
        passed("Pi namespace collisions between hyphens and underscores refuse native changes")

        project = record / "project";project.mkdir()
        project_source = cli("sources", "add", "--agent", "pi", "--scope", "project", "--project", str(project))
        project_path = Path(project_source["path"])
        assert project_path == project / ".pi/mcp.json" and not project_path.exists()
        cli("sources", "add", "--agent", "pi", "--scope", "project", "--project", str(project), "--path", str(path), fail=True)
        project_connection = cli("connections", "save", "--stdin", params={"name": "pi_project", "definition": {
            "url": harness.url, "headers": {"Authorization": "synthetic-fixture-authorization"}}})["id"]
        harness.rpc("connections.probe", {"id": project_connection})
        harness.changes(project_connection, [project_source["id"]], mode="shared")
        value = json.loads(project_path.read_text());value["mcpServers"]["pi_docs"] = {"enabled": False}
        project_path.write_text(json.dumps(value), encoding="utf-8")
        cli("connections", "import", "--source", project_source["id"], "--name", "pi_docs", fail=True)
        assert json.loads(project_path.read_text())["mcpServers"]["pi_docs"] == {"enabled": False}
        assert not (path.parent / "trust.json").exists()
        passed("explicit project sources use .pi/mcp.json and keep preference-only overrides and trust client-owned")

        local = cli("connections", "import", "--source", "codex:user", "--name", "local_notes")["id"]
        harness.changes(local, [source["id"]], mode="direct")
        http = cli("connections", "save", "--stdin", params={"name": "pi_http", "definition": {
            "url": harness.url, "headers": {"Authorization": "synthetic-fixture-authorization"}}})["id"]
        harness.changes(http, [source["id"]], mode="direct")
        passed("Pi accepts both exact STDIO and authenticated HTTP direct configurations")

        if args.pi_package:
            package = args.pi_package.resolve()
            native_env = {**harness.environment, "PI_CODING_AGENT_DIR": str(path.parent)}
            command = ["node", str(package / "dist/bundle/cli.js")]
            version = subprocess.check_output([*command, "--version"], env=native_env, text=True, timeout=30).strip()
            proc = subprocess.run([*command, "mcp", "list", "--json"], env=native_env, cwd=project,
                                  capture_output=True, text=True, timeout=60)
            (record / "pi-list.json").write_text(proc.stdout)
            assert proc.returncode == 0, (proc.stdout, proc.stderr)
            listing = json.loads(proc.stdout)
            assert not listing["errors"] and {s["name"] for s in listing["servers"]} == {"pi_docs", "pi_http", "local_notes"}
            assert all(s["state"] == "connected" for s in listing["servers"]), listing
            actual = []
            for trusted in (False, True):
                proc = subprocess.run(["node", str(ROOT / "tests/fixtures/pi-native-mcp.mjs"), str(package),
                                       str(path.parent), str(project), str(trusted).lower()], env=native_env,
                                      capture_output=True, text=True, timeout=60)
                assert proc.returncode == 0, (proc.stdout, proc.stderr)
                result = json.loads(proc.stdout);actual.append(result)
                if trusted:
                    assert next(s for s in result["servers"] if s["name"] == "pi_docs")["enabled"] is False
                    assert next(s for s in result["servers"] if s["name"] == "pi_project")["toolCall"] == "PASS"
                else:
                    assert "pi_project" not in {s["name"] for s in result["servers"]}
            native = {"conclusion": "PASS", "version": version, "cliList": "PASS", "runtimeCalls": actual,
                      "modelCalls": "NOT_RUN", "accountCredentials": "NOT_USED"}
            passed("official Pi CLI discovers the native entries; its published runtime calls direct and shared tools and honors project trust")

        harness.rpc("gateway.stop")
        harness.environment["PI_CODING_AGENT_DIR"] = "~/pi-custom-profile"
        custom = next(s for s in snapshot()["sources"] if s["agent"] == "pi" and Path(s["path"]) == harness.home / "pi-custom-profile/mcp.json")
        assert not Path(custom["path"]).exists()
        passed("PI_CODING_AGENT_DIR expands ~/ without scanning or modifying native login storage")
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(keep=False)
        (record / "report.json").write_text(json.dumps({"conclusion": conclusion, "error": error, "checks": checks,
            "nativePi": native, "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
            "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            "generatedAt": datetime.now(timezone.utc).isoformat()}, ensure_ascii=False, indent=2))
    if conclusion != "PASS":
        raise SystemExit(1)


if __name__ == "__main__":
    main()
