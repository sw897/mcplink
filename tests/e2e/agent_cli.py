"""Exercise public named CLI commands against isolated native configs and real SDK fixtures."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import tomllib
import traceback
import urllib.request

from run import BINARY, Harness, ROOT


class AgentCli(Harness):
    def __init__(self, record, **options):
        super().__init__(record, **options)
        self.commands = []

    def cli(self, *args, value=None, raw=None, exit_status=0, env=None):
        result = subprocess.run([str(BINARY), *args], input=json.dumps(value) if value is not None else raw or "",
                                env=env or self.environment, capture_output=True, text=True, timeout=90)
        self.commands.append({"arguments": ["[synthetic sentinel]" if a.startswith("synthetic-invalid-secret-") else a for a in args], "stdin": "JSON/input supplied" if value is not None or raw else None,
                              "exit_status": result.returncode, "stdout_sha256": hashlib.sha256(result.stdout.encode()).hexdigest()})
        assert result.returncode == exit_status, (args, result.returncode, result.stdout, result.stderr)
        assert not result.stderr, (args, result.stderr)
        response = json.loads(result.stdout)
        assert isinstance(response, dict)
        if exit_status == 0:
            assert "ok" in response, response
            return response["ok"]
        assert "error" in response, response
        return response

    def run(self):
        offline = self.record / "offline-data"
        schema = self.cli("--data-dir", str(offline), "schema")
        assert schema["schemaVersion"] == 1 and schema["executable"] == str(BINARY)
        assert {c["name"] for c in schema["command"]["commands"]} >= {"connections", "changes", "credentials", "skill"}
        skill = self.cli("--data-dir", str(offline), "skill", "show")
        assert skill["content"] == (ROOT / "skills/mcplink/SKILL.md").read_text()
        assert not offline.exists(), "offline commands must not start daemon or open Vault"
        self.passed("offline machine-readable CLI discovery and embedded Skill")

        secret = "synthetic-invalid-secret-sentinel-abcdefghijklmnop"
        for command, raw in [(("connections", "save", "--stdin"), '{"name":"' + secret),
                             (("connections", "save", "--stdin"), '[]'),
                             (("credentials", "update", "absent", "--stdin"), '{"value":""}'),
                             (("rpc", "--stdin"), '{}'),
                             (("connections", "save", "--stdin"), ' ' * (8 * 1024 * 1024 + 1))]:
            response = self.cli(*command, raw=raw, exit_status=2)
            assert response["error"]["code"] == "invalid_input"
            assert secret not in json.dumps(response)
        self.cli("connections", "save", exit_status=2)
        invalid = self.cli("unknown-command", secret, exit_status=2)
        assert secret not in json.dumps(invalid)
        response = self.cli("rpc", "changes.preview", "--params", "[]", exit_status=1)
        assert "panic" not in json.dumps(response)
        self.passed("invalid/missing/oversized stdin and arguments fail without secret echo or panic")

        status = self.cli("status")
        assert {s["agent"] for s in status["sources"]} == {"codex","hermes","claude","opencode","zcode","deepseek","openclaw","kimicode","workbuddy"} and len(status["connections"]) == 3
        assert self.cli("sources", "list") == status["sources"]
        local = self.cli("connections", "import", "--source", "codex:user", "--name", "local_notes")["id"]
        assert self.cli("connections", "probe", local)["connection"]["status"] == "ready"
        self.passed("scan and import native connection without editing its file")

        project = self.record / "project/config.toml"
        project.parent.mkdir()
        project.write_text('# project entry\nmodel = "keep-project"\n')
        source = self.cli("sources", "add", "--agent", "codex", "--path", str(project), "--label", "fixture project")
        assert Path(source["path"]).samefile(project) and source["id"].startswith("source:"), source
        saved = self.cli("connections", "save", "--stdin", value={"name": "cli_http", "title": "CLI test", "definition": {"url": self.url, "headers": {"Authorization": secret}}})
        connection = saved["id"]
        rejected = self.cli("connections", "probe", connection, exit_status=1)
        assert rejected["error"]["code"] == "connection_unready" and rejected["result"]["connection"]["status"] == "needs_auth"
        assert secret not in json.dumps(rejected)
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-fixture-authorization"})
        detail = self.cli("connections", "show", connection)
        assert detail["definition"]["headers"]["Authorization"] == "••••••••"
        assert "synthetic-fixture-authorization" not in json.dumps(detail)
        edited = {"id": connection, "name": "cli_http", "title": "CLI edited", "definition": detail["definition"]}
        self.cli("connections", "save", "--stdin", value=edited)
        tools = self.cli("connections", "tools", connection)
        assert len(tools["tools"]) == 2 and tools["connection"]["status"] == "ready"
        assert any(c["id"] == connection for c in self.cli("connections", "list"))
        self.passed("add project source, save/edit with masked credentials, reject invalid auth and recover via stdin")

        paths = [self.home / ".codex/config.toml", self.home / ".hermes/config.yaml"]
        before = [p.read_bytes() for p in paths]
        plan = self.cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--target", "hermes:user", "--mode", "shared")
        assert [p.read_bytes() for p in paths] == before
        assert len(plan["changes"]) == 2
        applied = self.cli("changes", "apply", plan["planId"])
        assert all(r["status"] == "applied" for r in applied["results"])
        native = tomllib.loads(paths[0].read_text())["mcp_servers"]["cli_http"]
        assert native["command"] == str(BINARY) and native["args"] == ["bridge", "--connection", connection]
        assert "synthetic-fixture-authorization" not in json.dumps(native)
        assert self.cli("rpc", "snapshot")["connections"] == self.cli("status")["connections"]
        bridge = self.bridge(connection, "agent-cli-e2e")
        assert len(bridge.call("tools/list")["result"]["tools"]) == 2
        self.passed("preview is read-only, apply binds two native Agents to current CLI and shares existing state")

        description = self.cli("mcp", "describe", connection)
        assert "tools" in description["capabilities"]
        listed = self.cli("mcp", "request", connection, "--stdin", value={"method": "tools/list"})
        assert len(listed["tools"]) == 2
        read = self.cli("mcp", "request", connection, "--stdin", value={"method": "tools/call", "params": {"name": "read_document", "arguments": {"id": "weekly-fixture"}}})
        assert "weekly-fixture" in json.dumps(read)
        failed = self.cli("mcp", "request", connection, "--stdin", value={"method": "tools/call", "params": {"name": "missing", "arguments": {}}}, exit_status=1)
        assert failed["error"]["code"] == "upstream_failed" and failed["result"]["isError"]
        self.cli("rpc", "unknown.method", exit_status=1)
        self.passed("MCP describe/request work, tool errors and management RPC failures exit nonzero")

        stale = self.cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--target", "hermes:user", "--action", "disable")
        paths[0].write_text(paths[0].read_text() + '\n# concurrent preserved\n')
        partial = self.cli("changes", "apply", stale["planId"], exit_status=1)
        assert partial["error"]["code"] == "partial_failure"
        assert [r["status"] for r in partial["result"]["results"]] == ["failed", "applied"]
        assert "# concurrent preserved" in paths[0].read_text()
        operation = partial["result"]["results"][1]["operationId"]
        paths[1].write_text(paths[1].read_text() + '\n# unrelated after apply\n')
        assert self.cli("changes", "restore", operation)["status"] == "restored"
        assert "# unrelated after apply" in paths[1].read_text()
        assert any(h["id"] == operation for h in self.cli("changes", "history"))
        self.passed("partial batch failure exits nonzero, protects concurrent edits and restores only the requested entry")

        codex = paths[0]
        codex.write_text(codex.read_text().replace('model = "fixture-model"', 'model = "keep-native-model"'))
        # Modify only the direct service, then explicitly adopt its newer native value.
        codex.write_text(codex.read_text().replace('[mcp_servers.local_notes]\n', '[mcp_servers.local_notes]\nenabled = false\n'))
        adopted = self.cli("connections", "adopt", "--source", "codex:user", "--name", "local_notes")
        assert adopted["id"] == local and not self.cli("connections", "show", local)["definition"]["enabled"]
        self.passed("explicitly adopt external direct-entry changes while retaining unrelated native settings")

        assert not self.cli("gateway", "status")["paused"]
        assert self.cli("gateway", "pause")["paused"]
        self.cli("mcp", "describe", connection, exit_status=1)
        assert not self.cli("gateway", "resume")["paused"]
        assert "result" in bridge.call("tools/list")
        self.passed("pause/resume shared Gateway through CLI and reuse existing Bridge")

        oauth = self.cli("connections", "save", "--stdin", value={"name": "cli_oauth", "definition": {"url": self.url, "oauth": True}})["id"]
        authorization = self.cli("credentials", "login", oauth)
        assert authorization["url"].startswith(f"http://127.0.0.1:{self.port}/authorize?")
        assert urllib.request.urlopen(authorization["url"], timeout=15).status == 200
        assert self.cli("connections", "probe", oauth)["connection"]["status"] == "ready"
        self.passed("OAuth returns authorization URL without opening UI and completes fixture callback")

        self.cli("connections", "delete", connection, exit_status=1)
        self.cli("credentials", "disconnect", connection)
        offline = self.cli("connections", "probe", connection, exit_status=1)
        assert offline["result"]["connection"]["status"] == "disconnected"
        remove = self.cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--target", "hermes:user", "--action", "remove")
        self.cli("changes", "apply", remove["planId"])
        assert self.cli("connections", "delete", connection)["deleted"]
        self.passed("disconnect marks unready, bound deletion is refused and explicit unbind permits deletion")

        for agent, directory in [("codex", self.home / ".codex"), ("hermes", self.home / ".hermes"), ("claude", self.home / ".claude")]:
            installed = self.cli("skill", "install", "--agent", agent)
            assert installed["status"] == "installed"
            skill = directory / "skills/mcplink/SKILL.md"
            assert skill.read_bytes() == (ROOT / "skills/mcplink/SKILL.md").read_bytes() and not skill.is_symlink()
            assert self.cli("skill", "install", "--agent", agent)["status"] == "unchanged"
        existing = self.home / ".codex/skills/mcplink/SKILL.md"
        existing.write_text("preserve different local Skill")
        self.cli("skill", "install", "--agent", "codex", exit_status=1)
        assert existing.read_text() == "preserve different local Skill"
        linked = self.home / ".hermes/skills/mcplink/SKILL.md"
        linked.unlink()
        linked.symlink_to(existing)
        self.cli("skill", "install", "--agent", "hermes", exit_status=1)
        assert existing.read_text() == "preserve different local Skill"
        self.passed("install embedded Skill independently on both Agents, idempotently preserve conflicts and reject symlinks")
        return {"managed_connection": connection, "local": local, "oauth": oauth}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = AgentCli(record)
    conclusion, error, ids = "PASS", None, {}
    try:
        ids = harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        print("FAIL", error, flush=True)
        traceback.print_exc()
    finally:
        harness.close(False)
    report = {"conclusion": conclusion, "error": error, "executed_at": datetime.now(timezone.utc).isoformat(),
              "os": platform.platform(), "command": "python3 tests/e2e/agent_cli.py --record <record>",
              "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(),
              "script_sha256": hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
              "skill_sha256": hashlib.sha256((ROOT / "skills/mcplink/SKILL.md").read_bytes()).hexdigest(),
              "fixture": "synthetic MCP/OAuth, isolated native config and fixture Vault", "scenarios": harness.results,
              "commands": harness.commands, "ids": ids, "real_native_mcp_config_modified": False}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(record, flush=True)
    raise SystemExit(0 if conclusion == "PASS" else 1)


if __name__ == "__main__":
    main()
