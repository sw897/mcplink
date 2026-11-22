"""Check configuration edits, write recovery and history through the installed CLI."""
import argparse
from contextlib import contextmanager
import getpass
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


def vault_file(h, key):
    return h.data / "fixture-vault" / hashlib.sha256(key.encode()).hexdigest()


@contextmanager
def hold_write_lock(path):
    with path.open("r+b") as handle:
        if os.name == "nt":
            import msvcrt
            msvcrt.locking(handle.fileno(), msvcrt.LK_NBLCK, 1)
            try:
                yield
            finally:
                msvcrt.locking(handle.fileno(), msvcrt.LK_UNLCK, 1)
        else:
            import fcntl
            fcntl.flock(handle, fcntl.LOCK_EX | fcntl.LOCK_NB)
            yield


def verify_storage(h, previous_binary):
    native = h.home / ".codex/config.toml"
    db_file = h.data / "mcplink.sqlite3"
    def sql(query, values=()):
        with sqlite3.connect(db_file) as db:
            return db.execute(query, values).fetchall()

    def cli(*args, **kwargs):
        return h.cli(*args, **kwargs)

    def preview(connection, action="upsert"):
        return cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--action", action)

    def save(name):
        return cli("connections", "save", "--stdin", value={"name":name,"definition":{
            "url":h.url,"headers":{"Authorization":"synthetic-fixture-authorization"}}})["id"]

    def apply(plan, expected=0):
        value = cli("changes", "apply", plan["planId"], exit_status=expected)
        return (value["result"] if expected else value)["results"][0]

    connection = save("storage_fixture")
    if sys.platform == "darwin":
        subprocess.run(["/bin/chmod", "+a", f"{getpass.getuser()} allow read,write", native], check=True)
        acl = subprocess.check_output(["/bin/ls", "-le", native], text=True).splitlines()[1:]
        assert acl
    if os.name == "nt":
        from windows_native_storage import powershell_storage

        def dacl(path):
            command = "(Get-Acl -LiteralPath $env:MCPLINK_STORAGE_TEST_PATH).Sddl"
            return powershell_storage(h.environment, command, path).split("D:", 1)[1]
        native_dacl = dacl(native)
    first = preview(connection)
    h.cli("gateway", "stop")
    row = apply(first)
    assert not vault_file(h, "plan/" + first["planId"]).exists() and apply(first) == row
    assert vault_file(h, "change/" + row["operationId"]).is_file()
    name = row["operationId"] + ".config"
    backup = h.data / "backups" / name
    assert backup.is_file() and cli("changes", "history")[0]["backupRetained"]
    if os.name == "posix":
        assert backup.stat().st_mode & 0o777 == 0o600
        assert db_file.stat().st_mode & 0o777 == 0o600
        assert backup.parent.stat().st_mode & 0o777 == 0o700
    if sys.platform == "darwin":
        assert subprocess.check_output(["/bin/ls", "-le", native], text=True).splitlines()[1:] == acl
    if os.name == "nt":
        result_dacl = dacl(native)
        before_flags, before_separator, before_rules = native_dacl.partition("(")
        after_flags, after_separator, after_rules = result_dacl.partition("(")
        # Win32 can mark an ordinary file's DACL as auto-inherited after setting it.
        # Protection and the complete ordered ACEs, including inheritance bits, must match.
        inherited_marker_added = before_flags == "" and after_flags == "AI"
        same_rules = (before_separator, before_rules) == (after_separator, after_rules)
        assert same_rules and (before_flags == after_flags or inherited_marker_added), (
            f"NTFS DACL changed after apply (flags {before_flags!r} to {after_flags!r}; "
            f"access rules unchanged: {same_rules})"
        )
        assert dacl(backup).startswith("P") and ";;;WD)" not in dacl(backup)
    h.passed("a preview survives restart, successful apply clears the plan secret, and the outcome and original file permissions remain")

    pending = [preview(connection, "disable") for _ in range(32)]
    assert "预览已满" in cli("changes", "preview", "--connection", connection, "--target", "codex:user", exit_status=1)["error"]["message"]
    sql("UPDATE change_previews SET expires_at=0 WHERE result IS NULL")
    cli("changes", "reconcile")
    assert all(not vault_file(h, "plan/" + plan["planId"]).exists() for plan in pending)
    assert "过期" in cli("changes", "apply", pending[0]["planId"], exit_status=1)["error"]["message"]
    assert apply(first) == row
    sql("UPDATE operations SET created_at=0 WHERE id=?", (row["operationId"],))
    cli("changes", "reconcile")
    assert not backup.exists() and not cli("changes", "history")[0]["backupRetained"]
    assert h.rpc("changes.preview_restore", {"operationId":row["operationId"]})["effect"] == "remove"
    h.passed("expired and over-capacity previews are bounded, and pruning an old whole-file backup preserves entry undo")

    second = h.record / "other-dataset"
    wanted = str(native.resolve())
    if sys.platform == "darwin" or os.name == "nt":
        wanted = wanted.lower()
    lock = h.home / ".mcplink/config-locks" / (hashlib.sha256(wanted.encode()).hexdigest()+".lock")
    try:
        other = cli("--data-dir", str(second), "connections", "save", "--stdin",
                    value={"name":"second_dataset","definition":{"command":"fixture"}})["id"]
        other_plan = cli("--data-dir", str(second), "changes", "preview", "--connection", other, "--target", "codex:user")
        first_plan = preview(connection, "disable")
        current = native.read_bytes()
        with hold_write_lock(lock):
            a=apply(first_plan, 1)
            b=cli("--data-dir", str(second), "changes", "apply", other_plan["planId"], exit_status=1)["result"]["results"][0]
            assert a["status"] == b["status"] == "failed" and all("另一个配置操作" in item["message"] for item in (a,b))
            assert native.read_bytes() == current
        assert not (native.parent/".mcplink.lock").exists()
    finally:
        cli("--data-dir", str(second), "gateway", "stop")
    h.passed("two independent data directories share a private lock for the same native file")

    def switch_fault(point):
        cli("gateway", "stop")
        if point is None:
            h.environment.pop("MCPLINK_FAILPOINT", None)
        else:
            h.environment["MCPLINK_FAILPOINT"] = point

    switch_fault("before-write")
    failed = apply(preview(connection, "disable"), 1)
    assert native.read_bytes() == current and failed["status"] == "failed"
    assert not vault_file(h,"change/"+failed["operationId"]).exists()
    assert not (h.data/"backups"/(failed["operationId"]+".config")).exists()
    assert sql("SELECT id FROM change_staging") == []
    switch_fault("config-sync")
    unsynced_plan = preview(connection, "disable")
    other_plan = preview(connection, "disable")
    pending_op = apply(unsynced_plan, 1)
    opid = pending_op["operationId"]
    assert pending_op["status"] == "pending" and not tomllib.loads(native.read_text())["mcp_servers"]["storage_fixture"]["enabled"]
    snapshot = vault_file(h, "change/" + opid)
    original = snapshot.read_bytes()
    snapshot.write_text("synthetic broken transaction record")
    switch_fault(None)
    assert cli("changes", "history")[0]["status"] == "pending"
    assert "损坏" in cli("changes", "history")[0]["error"]
    sql("UPDATE operations SET created_at=0 WHERE id=?", (opid,))
    assert cli("changes", "reconcile")["remaining"] == 1
    assert (h.data / "backups" / (opid+".config")).exists()
    assert cli("connections", "probe", connection)["connection"]["status"] == "ready"
    assert "result" in h.bridge(connection, "isolated-recovery").call("tools/list")
    blocked = apply(other_plan, 1)
    assert "待核对" in blocked["message"]
    assert "待核对" in cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--action", "enable", exit_status=1)["error"]["message"]
    snapshot.unlink()
    assert cli("changes", "reconcile")["remaining"] == 1
    snapshot.mkdir()
    assert cli("changes", "reconcile")["remaining"] == 1
    assert "无法读取" in cli("changes", "history")[0]["error"]
    snapshot.rmdir()
    source = sql("SELECT source_id FROM operations WHERE id=?", (opid,))[0][0]
    sql("UPDATE operations SET source_id='fixture:missing-source' WHERE id=?", (opid,))
    snapshot.write_text(json.dumps(json.loads(original) | {"sourceId":"fixture:missing-source"}))
    assert cli("changes", "reconcile")["remaining"] == 1
    assert "来源不可用" in cli("changes", "history")[0]["error"]
    sql("UPDATE operations SET source_id=? WHERE id=?", (source,opid))
    snapshot.write_bytes(original)
    assert cli("changes", "reconcile")["remaining"] == 0
    assert apply(unsynced_plan)["status"] == "applied" and h.rpc("changes.preview_restore", {"operationId":opid})["effect"] == "update"
    assert not (h.data / "backups" / (opid+".config")).exists()
    h.passed("pre-write artifacts are removed, while post-replacement faults keep per-entry write protection and healthy Bridge access until recovery")

    if previous_binary is not None:
        def previous(*args, value=None):
            result = subprocess.run([str(previous_binary), *args], env=h.environment, input=json.dumps(value) if value is not None else "", text=True,capture_output=True,timeout=30)
            assert result.returncode==0, (args,result.stdout,result.stderr)
            return json.loads(result.stdout)["ok"]
        cli("gateway", "stop")
        old_id = previous("connections", "save", "--stdin", value={"name":"older_preview","definition":{"command":"fixture"}})["id"]
        saved_args = ("changes", "preview", "--connection", old_id, "--target", "codex:user")
        old_applied = previous(*saved_args)
        old_expired = previous(*saved_args)
        previous("gateway", "stop")
        assert vault_file(h,"plan/"+old_expired["planId"]).exists()
        old_op = apply(old_applied)["operationId"]
        sql("UPDATE settings SET value='0' WHERE name='changes.legacy-sweep'")
        cli("changes", "reconcile")
        assert not vault_file(h,"plan/"+old_expired["planId"]).exists()
        assert vault_file(h,"change/"+old_op).exists()
        assert cli("changes", "restore", old_op)["status"] == "restored"
        h.passed("an older build's Vault preview is usable during migration and scoped cleanup leaves committed undo intact")


def verify_missing_binding(h):
    native = h.home / ".hermes/config.yaml"
    original = native.read_bytes()
    connection = h.cli("connections", "save", "--stdin", value={"name":"repair_docs", "definition":{
        "url":h.url, "headers":{"Authorization":"synthetic-fixture-authorization"}}})["id"]
    h.cli("connections", "probe", connection)

    def preview(mode):
        return h.cli("changes", "preview", "--connection", connection, "--target", "hermes:user", "--mode", mode)

    for mode in ("direct", "shared"):
        h.changes(connection, ["hermes:user"], mode=mode)
        configured = native.read_text()
        assert "  repair_docs:" in configured
        renamed = configured.replace("  repair_docs:", f"  externally_renamed_{mode}:", 1)
        native.write_text(renamed)
        assert any(b.get("connectionId")==connection and b.get("missing") for b in h.rpc("snapshot")["bindings"])
        plan = preview(mode)
        assert plan["changes"][0]["kind"] == "add" and plan["changes"][0]["name"] == "repair_docs"
        concurrent = renamed + "\n# external edit after preview\n"
        native.write_text(concurrent)
        failed = h.cli("changes", "apply", plan["planId"], exit_status=1)
        assert failed["result"]["results"][0]["status"] == "failed" and native.read_text()==concurrent
        applied = h.cli("changes", "apply", preview(mode)["planId"])["results"][0]
        assert applied["status"] == "applied"
        snapshot = h.rpc("snapshot")
        binding = next(b for b in snapshot["bindings"] if b.get("connectionId")==connection)
        assert not binding.get("missing") and not binding["changed"] and binding["mode"]==mode
        assert any(b["name"]==f"externally_renamed_{mode}" and b.get("connectionId") is None for b in snapshot["bindings"])
        if mode == "shared":
            assert "result" in h.bridge(connection, "restored-shared-binding").call("tools/list")
        h.passed(f"explicit {mode} reattachment recreates only the missing entry and rejects edits after preview")

    configured = native.read_text()
    aliased = configured.replace("  repair_docs:", "  bound_alias:", 1)
    native.write_text(aliased)
    assert h.cli("connections", "import", "--source", "hermes:user", "--name", "bound_alias")["id"] == connection
    native.write_text(aliased.replace("  bound_alias:", "  retained_alias:", 1))
    params = {"connectionId":connection,"targetSourceIds":["hermes:user"],"mode":"shared",
              "bindingTarget":{"sourceId":"hermes:user","name":"bound_alias"}}
    plan = h.rpc("changes.preview", params)
    assert plan["changes"][0]["name"] == "bound_alias"
    assert h.cli("changes", "apply", plan["planId"])["results"][0]["status"] == "applied"
    assert "  bound_alias:" in native.read_text() and "  repair_docs:" not in native.read_text()
    cleanup = h.rpc("changes.preview", {**params,"action":"remove"})
    h.cli("changes", "apply", cleanup["planId"])
    native.write_text(configured)
    native.unlink()
    assert h.cli("changes", "apply", preview("shared")["planId"])["results"][0]["status"] == "applied"
    recreated = next(b for b in h.rpc("snapshot")["bindings"] if b.get("connectionId")==connection)
    assert native.exists() and recreated["name"]=="repair_docs" and not recreated.get("missing")
    native.write_text(configured)
    h.passed("explicit reattachment retains the selected binding alias and can recreate a missing file")

    configured = native.read_text()
    changed = json.dumps({"mcp_servers":{"repair_docs":{"command":"/synthetic/changed-cli","args":["--changed"]}}})
    native.write_text(changed)
    error = h.cli("changes", "preview", "--connection", connection, "--target", "hermes:user", exit_status=1)
    assert "外部修改" in error["error"]["message"] and native.read_text()==changed
    native.write_text("mcp_servers: [broken\n")
    h.cli("changes", "preview", "--connection", connection, "--target", "hermes:user", exit_status=1)
    assert native.read_text()=="mcp_servers: [broken\n"
    native.write_text(configured)
    h.changes(connection, ["hermes:user"], action="remove")
    h.cli("connections", "delete", connection)
    native.write_bytes(original)
    h.passed("reattachment still rejects externally changed existing entries and unreadable sources")


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


def verify_source_identity(h):
    if os.name != "posix":
        return
    alias = h.home / ".kimi-code"
    real = h.record / "kimi config"
    real.mkdir()
    alias.symlink_to(real, target_is_directory=True)
    source = next(s for s in h.cli("sources", "list") if s["agent"] == "kimicode")
    connection = h.cli("connections", "save", "--stdin", value={"name": "first_file", "definition": {"url": h.url}})["id"]
    plan = h.cli("changes", "preview", "--connection", connection, "--target", source["id"])
    applied = h.cli("changes", "apply", plan["planId"])["results"][0]
    chosen = next(s for s in h.cli("sources", "list") if s["agent"] == "kimicode")
    assert chosen["id"] == source["id"] and Path(chosen["path"]) == real / "mcp.json"
    h.passed("first creation below a directory alias preserves source and binding identity")

    operation = applied["operationId"]
    legacy = "native:kimicode:" + hashlib.sha256(str(alias / "mcp.json").encode()).hexdigest()
    assert legacy != source["id"]
    h.cli("gateway", "stop")
    # Match the records the previous build left after writing through the alias
    # and before committing any binding or ownership update.
    reference = h.data / "fixture-vault" / hashlib.sha256(f"change/{operation}".encode()).hexdigest()
    original = json.loads(reference.read_text())
    reference.write_text(json.dumps(original | {"sourceId": legacy}))
    with sqlite3.connect(h.data / "mcplink.sqlite3") as db:
        db.execute("DELETE FROM bindings WHERE source_id=? AND name='first_file'", (source["id"],))
        db.execute("DELETE FROM entry_ownership WHERE source_id=? AND name='first_file'", (source["id"],))
        db.execute("UPDATE operations SET source_id=?,status='pending' WHERE id=?", (legacy, operation))
    status = h.cli("status")
    binding = next(b for b in status["bindings"] if b.get("connectionId") == connection)
    assert binding["sourceId"] == legacy and not binding.get("changed") and not binding.get("missing")
    assert h.cli("changes", "restore", operation)["status"] == "restored"
    config = real / "mcp.json"
    assert "first_file" not in json.loads(config.read_text())["mcpServers"]
    config.unlink()
    unrelated = h.record / "other-config.json"
    unrelated.write_text('{"keep": true}\n')
    config.symlink_to(unrelated)
    try:
        rejected = h.cli("changes", "preview", "--connection", connection, "--target", legacy, exit_status=1)
        assert "符号链接" in rejected["error"]["message"] and json.loads(unrelated.read_text()) == {"keep": True}
    finally:
        config.unlink()
    h.passed("legacy source ID reconciles interrupted writes and restores history while preserving the final link guard")


def verify_scoped_rendering(h):
    codex = h.home / ".codex/config.toml"
    initial = codex.read_text()
    codex.write_text(initial + f'\n[mcp_servers.render_fixture]\nurl = {json.dumps(h.url)}\n'
                     'http_headers = {Authorization = "synthetic-fixture-authorization"}\n'
                     'fixture_extension = "original-transport"\nrequired = true\n'
                     'default_tools_approval_mode = "prompt"\n'
                     '[mcp_servers.render_fixture.tools.read_document]\napproval_mode = "prompt"\n')
    direct = tomllib.loads(codex.read_text())["mcp_servers"]["render_fixture"]
    connection = h.cli("connections", "import", "--source", "codex:user", "--name", "render_fixture")["id"]
    h.cli("connections", "probe", connection)
    def convert():
        plan = h.cli("changes", "preview", "--connection", connection, "--target", "codex:user", "--mode", "shared")
        return h.cli("changes", "apply", plan["planId"])["results"][0]["operationId"]

    operation = convert()
    raw = tomllib.loads(codex.read_text())["mcp_servers"]["render_fixture"]
    assert "fixture_extension" not in raw and "http_headers" not in raw
    assert raw["required"] and raw["default_tools_approval_mode"] == "prompt" and raw["tools"] == direct["tools"]
    assert '# Codex user comment' in codex.read_text()
    row = next(b for b in h.cli("status")["bindings"] if b.get("connectionId") == connection)
    assert row["mode"] == "shared" and not row["changed"]
    assert h.cli("changes", "restore", operation)["status"] == "restored"
    assert tomllib.loads(codex.read_text())["mcp_servers"]["render_fixture"] == direct
    operation = convert()
    # Simulate an earlier writer that left the original extension on disk.
    exact = codex.read_text()
    with_residue = exact.replace("[mcp_servers.render_fixture]\n", '[mcp_servers.render_fixture]\nfixture_extension = "original-transport"\n')
    assert with_residue != exact
    codex.write_text(with_residue.replace("original-transport", "unexpected-change"))
    h.cli("changes", "restore", operation, exit_status=1)
    codex.write_text(with_residue)
    assert h.cli("changes", "restore", operation)["status"] == "restored"
    assert tomllib.loads(codex.read_text())["mcp_servers"]["render_fixture"] == direct
    h.passed("Codex conversion preserves native policies, restores the exact old entry and accepts only provable historical residue")

    hermes = h.home / ".hermes/config.yaml"
    hermes.write_text(hermes.read_text() + f'  render_yaml:\n    url: {h.url}\n'
                      '    headers: {Authorization: synthetic-fixture-authorization}\n'
                      '    fixture_extension: original-transport\n    tools: {resources: false}\n')
    yaml_connection = h.cli("connections", "import", "--source", "hermes:user", "--name", "render_yaml")["id"]
    h.cli("connections", "probe", yaml_connection)
    plan = h.cli("changes", "preview", "--connection", yaml_connection, "--target", "hermes:user", "--mode", "shared")
    operation = h.cli("changes", "apply", plan["planId"])["results"][0]["operationId"]
    assert "fixture_extension" not in hermes.read_text() and "resources: false" in hermes.read_text()
    with sqlite3.connect(h.data / "mcplink.sqlite3") as db:
        db.execute("UPDATE operations SET status='pending' WHERE id=?", (operation,))
    h.cli("gateway", "stop")
    assert next(row for row in h.cli("changes", "history") if row["id"] == operation)["status"] == "applied"
    assert h.cli("changes", "restore", operation)["status"] == "restored"
    assert "fixture_extension: original-transport" in hermes.read_text() and "# Hermes user comment" in hermes.read_text()
    h.passed("Hermes conversion and interrupted bookkeeping compare the written entry and preserve unrelated comments")


def verify(h):
    verify_source_identity(h)
    verify_scoped_rendering(h)
    verify_missing_binding(h)
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
    save("local", edit, id=local)
    assert apply(preview(local))["name"] == "local" and entry("local")["args"] == argv
    original = edit["args"]
    edit["args"] = [original[2], "prefix", original[3], original[0], original[1]]
    h.cli("connections", "save", "--stdin", value={"name":"remote","id":http,"definition":edit}, exit_status=1)
    changed = h.cli("connections", "save", "--stdin", value={"name":"local","id":local,"definition":edit}, exit_status=1)
    assert "重新填写受保护的启动参数" in changed["error"]["message"] and entry("local")["args"] == argv
    edit["args"] = ["", "prefix", "--stdio", "--api_key", "abc!"]
    save("local", edit, id=local)
    h.cli("connections", "save", "--stdin", value={"name":"local","id":local,"definition":edit}, exit_status=1)
    assert apply(preview(local))["name"] == "local"
    assert entry("local")["args"] == ["", "prefix", "--stdio", "--api_key", "abc!"]
    h.passed("numbered hidden arguments preserve empty values for the same launch; changed arguments require re-entry and stale/foreign versions fail")

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
    with sqlite3.connect(h.data / "mcplink.sqlite3") as db:
        future_schema = db.execute("PRAGMA user_version").fetchone()[0] + 1
    fault(f"PRAGMA user_version={future_schema}")
    refused = subprocess.run([str(BINARY),"daemon"], env=h.environment, capture_output=True, timeout=10)
    assert refused.returncode != 0
    with sqlite3.connect(h.data / "mcplink.sqlite3") as db:
        assert db.execute("PRAGMA user_version").fetchone()[0] == future_schema
    h.passed("opening a newer database fails without downgrading its schema marker")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--previous-binary", type=Path)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True)
    harness = AgentCli(record)
    report = {"binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(), "conclusion":"FAIL"}
    storage = None
    try:
        verify(harness)
        storage_record = record / "storage"
        storage_record.mkdir()
        storage = AgentCli(storage_record)
        verify_storage(storage, args.previous_binary.resolve() if args.previous_binary else None)
        report["conclusion"] = "PASS"
    finally:
        harness.close(False)
        if storage is not None:
            storage.close(False)
        report["scenarios"] = harness.results + (storage.results if storage else [])
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
