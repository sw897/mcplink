"""Check configuration edits, write recovery and history through the installed CLI."""
import argparse
import hashlib
import http.server
import threading
import json
import os
from pathlib import Path
import subprocess
import sqlite3
import sys
import tomllib

from agent_cli import AgentCli
from run import BINARY


def verify_origin(h):
    received = []
    class Endpoint(http.server.BaseHTTPRequestHandler):
        def do_POST(self):
            self.rfile.read(int(self.headers.get("Content-Length", 0)))
            received.append(self.headers.get("Authorization"))
            self.send_response(403)
            self.end_headers()

        def log_message(self, *args):
            pass

    with http.server.ThreadingHTTPServer(("127.0.0.1", 0), Endpoint) as endpoint:
        threading.Thread(target=endpoint.serve_forever, daemon=True).start()
        try:
            connection = h.rpc("connections.save", {"name": "move_origin", "definition": {
                "url": h.url, "headers": {"Authorization": "synthetic-fixture-authorization"}}})["id"]
            h.rpc("connections.probe", {"id": connection})
            h.changes(connection, ["hermes:user"], mode="shared")
            bridge = h.bridge(connection, "origin-check")
            assert "result" in bridge.call("tools/list")
            definition = h.rpc("connections.detail", {"id": connection})["definition"]
            definition["url"] = f"http://127.0.0.1:{endpoint.server_port}/mcp"
            result = h.rpc("connections.save", {"id": connection, "name": "move_origin", "definition": definition})
            assert not result["active"] and "error" in bridge.call("tools/list") and received == []
            h.cli("connections", "reconnect", connection)
            assert "error" in bridge.call("tools/list") and received and not any(received)
            h.rpc("credentials.update", {"id": connection, "value": "synthetic-new-origin"})
            bridge.call("tools/list")
            assert received[-1] == "synthetic-new-origin" and "synthetic-fixture-authorization" not in received
        finally:
            endpoint.shutdown()
    h.passed("existing Bridge blocks a changed origin until reconnect and never sends the old header")


def verify(h):
    verify_origin(h)
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
    pending_file = native.read_bytes()
    refused = h.cli("connections", "delete", local, "--keep-native", exit_status=1)
    assert "待核对" in refused["error"]["message"] and native.read_bytes() == pending_file
    assert h.cli("connections", "show", local)["connection"]["id"] == local
    h.passed("keep-native preserves the connection and file while an earlier config write awaits recovery")
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

    # Same bytes under a new owner must invalidate both restore and old previews.
    first_owner = save("replacement", {"url": h.url}, title="First owner")
    original_op = apply(preview(first_owner))["operationId"]
    apply(preview(first_owner, "remove"))
    second_owner = save("replacement", {"url": h.url}, title="Second owner")
    apply(preview(second_owner))
    for method in ("changes.preview_restore", "changes.restore"):
        assert "后续修改" in h.rpc(method, {"operationId": original_op}, error=True)
    assert entry("replacement")["url"] == h.url
    assert any(b.get("connectionId") == second_owner for b in h.rpc("snapshot")["bindings"])
    stale_plan = preview(second_owner, "remove")
    h.cli("connections", "detach", second_owner, "--source", "codex:user", "--name", "replacement")
    assert entry("replacement")["url"] == h.url
    assert h.rpc("changes.apply", {"planId": stale_plan["planId"]})["results"][0]["status"] == "failed"
    h.passed("same-name replacement keeps the new owner and native entry; detached plans expire")

    absent = next(s for s in h.rpc("snapshot")["sources"] if s["agent"] == "kimicode" and not s["exists"])
    empty = h.rpc("changes.preview", {"connectionId": second_owner, "targetSourceIds": [absent["id"]], "action": "remove"})
    assert not empty["changes"] and not h.rpc("changes.apply", {"planId": empty["planId"]})["results"]
    assert not Path(absent["path"]).exists()
    h.passed("removing an absent entry never creates a native file")

    auth = save("private_docs", {"url": h.url, "headers": {"Authorization": "synthetic-fixture-authorization"}})
    apply(preview(auth))
    h.rpc("credentials.update", {"id": auth, "value": "synthetic-second"})
    diff = preview(auth)["changes"][0]
    assert diff["before"] == diff["after"] and diff["fields"] == [{"field": "headers", "key": "Authorization", "kind": "update"}]
    assert "synthetic-second" not in json.dumps(diff)
    before = h.rpc("credentials.disconnect", {"id": auth})
    old = h.cli("connections", "show", auth)["definition"]
    save("private_docs", old, id=auth, title="Archived")
    after = h.cli("connections", "show", auth)["connection"]
    assert not after["active"] and after["status"] == "disconnected" and after["revision"] == before["revision"]
    h.cli("connections", "reconnect", auth)
    assert h.cli("connections", "show", auth)["connection"]["active"]
    h.passed("field diff names rotated secrets without exposing them; rename preserves the disconnected state")

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
