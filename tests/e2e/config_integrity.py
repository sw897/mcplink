"""Check configuration edits, write recovery and history through the installed CLI."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
import tomllib

from agent_cli import AgentCli
from run import BINARY


def verify(h):
    native = h.home / ".codex/config.toml"

    def save(name, definition, **metadata):
        return h.cli("connections", "save", "--stdin", value={
            "name": name, "definition": definition, **metadata})["id"]

    def preview(connection, action="upsert"):
        return h.cli("changes", "preview", "--connection", connection, "--target", "codex:user",
                     "--action", action)

    def apply(plan):
        return h.cli("changes", "apply", plan["planId"])["results"][0]

    def entry(name):
        return tomllib.loads(native.read_text())["mcp_servers"][name]

    url = h.url + "?tenant=demo&token=synthetic"
    http = save("remote", {"url": url})
    edit = h.cli("connections", "show", http)["definition"]
    assert "tenant=" not in json.dumps(edit)
    save("remote", edit, id=http, title="New title")
    if os.name == "posix":
        native.chmod(0o640)
    if sys.platform == "darwin":
        subprocess.run(["xattr", "-w", "io.mcplink.fixture", "retain", native], check=True)
    assert apply(preview(http))["status"] == "applied"
    assert entry("remote")["url"] == url
    if os.name == "posix":
        assert native.stat().st_mode & 0o777 == 0o640
    if sys.platform == "darwin":
        assert subprocess.check_output(["xattr", "-p", "io.mcplink.fixture", native]).strip() == b"retain"
    h.passed("rename preserves exact query and existing access mode / macOS extended attributes")

    argv = ["--api_key", "abc!", "", "--stdio"]
    local = save("local", {"command": sys.executable, "args": argv})
    edit = h.cli("connections", "show", local)["definition"]
    assert "abc!" not in json.dumps([edit, preview(local)])
    original = edit["args"]
    edit["args"] = [original[2], "prefix", original[3], original[0], original[1]]
    h.cli("connections", "save", "--stdin", value={"name":"remote","id":http,"definition":edit}, exit_status=1)
    save("local", edit, id=local)
    h.cli("connections", "save", "--stdin", value={"name":"local","id":local,"definition":edit}, exit_status=1)
    assert apply(preview(local))["name"] == "local"
    assert entry("local")["args"] == ["", "prefix", "--stdio", "--api_key", "abc!"]
    h.passed("numbered hidden arguments preserve empty values and reorder safely; stale/foreign references fail")

    def fault(sql):
        with sqlite3.connect(h.data / "mcplink.sqlite3") as db:
            db.execute(sql)

    install_fault = "CREATE TRIGGER fixture_fault BEFORE INSERT ON bindings BEGIN SELECT RAISE(ABORT,'fixture'); END"
    fault(install_fault)
    plan = preview(local, "disable")
    result = h.cli("changes", "apply", plan["planId"], exit_status=1)["result"]["results"][0]
    assert result["status"] == "pending" and not entry("local")["enabled"]
    h.cli("gateway", "stop")
    assert h.cli("changes", "history")[0]["status"] == "pending"
    assert h.cli("changes","reconcile")["remaining"] == 1
    fault("DROP TRIGGER fixture_fault")
    assert h.cli("changes","reconcile")["remaining"] == 0
    assert apply(plan)["status"] == "applied"
    fault(install_fault)
    h.cli("changes", "restore", result["operationId"], exit_status=1)
    assert entry("local")["enabled"] and h.cli("changes", "history")[0]["status"] == "pending"
    fault("DROP TRIGGER fixture_fault")
    h.cli("gateway", "stop")
    assert h.cli("changes", "history")[0]["status"] == "applied"
    assert h.cli("changes", "restore", result["operationId"])["status"] == "restored"
    h.passed("persistent binding failure stays pending and recoverable; interrupted restore commits both history rows")

    first = h.cli("changes", "history")[-1]["id"]
    for i in range(26):
        apply(preview(local, "disable" if i % 2 == 0 else "enable"))
    recent = h.cli("changes", "history")
    earlier = h.cli("changes", "history", "--before", str(recent[-1]["cursor"]))
    assert len(recent) == 25 and first in [op["id"] for op in earlier]
    assert not {op["id"] for op in recent}.intersection(op["id"] for op in earlier)
    assert not any("credentialRef" in op or "backup" in op for op in recent + earlier)
    h.passed("history cursor exposes older operations without duplicates or private storage references")

    launcher = h.launchers["codex"]
    pinned = h.home / "custom" / launcher.name
    pinned.parent.mkdir()
    launcher.rename(pinned)
    assert "未安装" in h.rpc("changes.preview", {"connectionId":local,"targetSourceIds":["codex:user"]},error=True)
    h.rpc("agents.launcher", {"agent":"codex","path":str(pinned)})
    preview(local)
    if os.name == "posix":
        nvm = h.home / ".nvm/versions/node/v22.0.0/bin/codex"
        nvm.parent.mkdir(parents=True)
        pinned.rename(nvm)
        preview(local)
        nvm.chmod(0o600)
        assert "未安装" in h.rpc("changes.preview", {"connectionId":local,"targetSourceIds":["codex:user"]},error=True)
        nvm.chmod(0o755)
        plan = preview(local, "disable")
        redirect = native.with_suffix(".original")
        native.rename(redirect)
        native.symlink_to(redirect)
        assert h.cli("changes", "apply", plan["planId"],exit_status=1)["result"]["results"][0]["status"] == "failed"
        assert native.is_symlink() and entry("local")["enabled"]
    h.passed("registered launchers are rechecked and nvm is discovered on Unix; symlink swaps refuse writes")

    h.cli("gateway", "stop")
    fault("PRAGMA user_version=2")
    refused = subprocess.run([str(BINARY),"daemon"], env=h.environment, capture_output=True, timeout=10)
    assert refused.returncode != 0
    with sqlite3.connect(h.data / "mcplink.sqlite3") as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == 2
    h.passed("opening a newer database fails without downgrading its schema marker")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True)
    harness = AgentCli(record)
    report = {"binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(), "conclusion":"FAIL"}
    try:
        verify(harness)
        report["conclusion"] = "PASS"
    finally:
        harness.close(False)
        report["scenarios"] = harness.results
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
