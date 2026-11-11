"""Characterize catalog limits and credential transitions at the public CLI/Bridge."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import sqlite3
import subprocess
import time
import traceback
import urllib.request

from tool_calls import ToolCalls
from run import BINARY


class ArchitectureBoundaries(ToolCalls):
    def control(self, gate, action):
        request = urllib.request.Request(f"http://127.0.0.1:{self.port}/control",
            data=json.dumps({"gate":gate,"action":action}).encode(),
            headers={"Content-Type":"application/json"})
        with urllib.request.urlopen(request, timeout=5) as response:
            assert json.load(response)["ok"]

    def wait_gate(self, name, previous, timeout=10):
        deadline = time.monotonic()+timeout
        while time.monotonic() < deadline:
            if self.stats()["gates"].get(name,0) > previous: return
            time.sleep(0.02)
        raise AssertionError("fixture gate not reached: "+name)

    def catalog(self):
        healthy = self.save("healthy", url=self.url)
        for mode in ["cycle", "duplicate", "empty", "cursor"]:
            bad = self.save("bad_"+mode, url=f"http://127.0.0.1:{self.port}/mcp-catalog-{mode}")
            direct = self.invoke(bad,"item",{},exit_status=1)
            assert direct["error"]["code"] == "tool_discovery_failed"
            entry = self.cli("gateway","save","--stdin",value={"connectionIds":[bad,healthy]})
            bridge = self.bridge(entry["id"],"malformed-"+mode)
            for method, field, key in [("tools/list","tools","name"),
                    ("resources/list","resources","uri"),
                    ("resources/templates/list","resourceTemplates","uriTemplate"),
                    ("prompts/list","prompts","name")]:
                page = bridge.call(method)["result"]
                assert page[field] and all(row[key] for row in page[field])
                assert [error["connectionId"] for error in page["_meta"]["mcplink"]["upstreamErrors"]] == [bad]
            alias = next(m["namespace"] for m in entry["members"] if m["connectionId"] == healthy)+"__read_document"
            assert "result" in bridge.call("tools/call",{"name":alias,"arguments":{"id":"weekly-fixture"}})
        self.passed("all four catalog families isolate cyclic, duplicate, empty and invalid-cursor members while healthy tools still execute")

        boundary = self.save("exact_boundary",url=f"http://127.0.0.1:{self.port}/mcp-catalog-bounds")
        assert len(self.cli("connections","tools",boundary)["tools"]) == 10_000
        for mode in ["pages", "items"]:
            over = self.save("over_"+mode,url=f"http://127.0.0.1:{self.port}/mcp-catalog-{mode}")
            assert self.invoke(over,"0",{},exit_status=1)["error"]["code"] == "tool_discovery_failed"
        self.passed("exactly 100 pages and 10000 tools are accepted; extra pages or tools fail closed")

        self.cli("gateway","pause")
        try:
            assert self.cli("connections","probe",healthy)["connection"]["status"] == "ready"
            self.invoke(healthy,"read_document",{"id":"weekly-fixture"},exit_status=1)
        finally: self.cli("gateway","resume")
        self.passed("paused shared traffic stays blocked while isolated management probes remain available")

    def oauth(self, name):
        connection = self.save(name,url=self.url)
        authorization = self.cli("credentials","login",connection)
        assert urllib.request.urlopen(authorization["url"],timeout=15).status == 200
        return connection

    def persistence(self):
        connection = self.oauth("persistence")
        grant = self.data/"fixture-vault"/hashlib.sha256(("oauth/"+connection).encode()).hexdigest()
        before = self.cli("connections","show",connection)
        saved_grant = grant.read_bytes()
        database = self.data/"mcplink.sqlite3"
        with sqlite3.connect(database) as db:
            db.execute("CREATE TRIGGER fixture_credential_fault BEFORE INSERT ON connections BEGIN SELECT RAISE(ABORT,'fixture'); END")
        try:
            self.cli("credentials","oauth-config",connection,"--stdin",value={"scopes":["documents:write"]},exit_status=1)
            assert self.cli("connections","show",connection) == before
            assert grant.is_file() and grant.read_bytes() == saved_grant, "failed definition commit destroyed the old OAuth grant"
        finally:
            with sqlite3.connect(database) as db: db.execute("DROP TRIGGER fixture_credential_fault")
        assert not self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        self.passed("SQLite rejection preserves the prior definition and usable OAuth grant")
        saved_grant = grant.read_bytes()

        # A filesystem directory deterministically rejects remove_file on all
        # supported platforms, without changing the OS Vault or its permissions.
        grant.unlink()
        grant.mkdir()
        try:
            failure = self.cli("credentials","oauth-config",connection,"--stdin",value={"scopes":["documents:write"]},exit_status=1)
            assert "已保存" in failure["error"]["message"], "post-commit cleanup failure hid the committed configuration"
            changed = self.cli("connections","show",connection)
            assert changed["definition"]["oauthScopes"] == ["documents:write"]
            assert changed["connection"]["status"] == "needs_auth"
        finally:
            grant.rmdir()
            grant.write_bytes(saved_grant)
        before_calls = self.stats()["calls"]
        self.invoke(connection,"read_document",{"id":"weekly-fixture"},exit_status=1)
        assert self.stats()["calls"] == before_calls, "a cleanup failure reused the superseded grant"
        self.passed("post-commit cleanup failure reports the applied configuration and refuses a retained superseded grant")

    def refresh(self):
        connection = self.oauth("refresh_boundary")
        self.cli("gateway","pause")  # Forces the probe to use a disposable pool.
        previous = self.stats()["gates"].get("refresh_token",0)
        self.control("refresh_token","arm")
        try:
            with ThreadPoolExecutor(max_workers=2) as workers:
                probe = workers.submit(subprocess.run,[str(BINARY),"connections","probe",connection],
                    env=self.environment,capture_output=True,text=True,timeout=30)
                self.wait_gate("refresh_token",previous)
                update = workers.submit(self.cli,"credentials","oauth-config",connection,"--stdin",value={"scopes":["documents:write"]})
                time.sleep(0.3)
                waiting = not update.done()
                self.control("refresh_token","release")
                response = probe.result(timeout=30)
                assert response.returncode in [0,1] and not response.stderr
                update.result(timeout=30)
                assert waiting, "OAuth configuration committed while an old grant refresh was still in flight"
        finally:
            self.control("refresh_token","release")
            self.cli("gateway","resume")
        assert self.cli("connections","show",connection)["connection"]["status"] == "needs_auth"
        self.invoke(connection,"read_document",{"id":"weekly-fixture"},exit_status=1)
        self.passed("OAuth settings serialize with disposable-pool refresh and revoke the old scope grant before later calls")

    def callback(self):
        connection = self.save("callback_commit",url=self.url)
        authorization = self.cli("credentials","login",connection)
        with sqlite3.connect(self.data/"mcplink.sqlite3") as db:
            db.execute("CREATE TRIGGER fixture_callback_fault BEFORE INSERT ON connections BEGIN SELECT RAISE(ABORT,'fixture'); END")
        try:
            try:
                response = urllib.request.urlopen(authorization["url"],timeout=15)
                status = response.status
                response.close()
            except urllib.error.HTTPError as error:
                status = error.code
                error.close()
            assert status == 503, "callback reported success after its status commit was rejected: "+str(status)
        finally:
            with sqlite3.connect(self.data/"mcplink.sqlite3") as db: db.execute("DROP TRIGGER fixture_callback_fault")
        assert not self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        self.passed("a callback reports an unavailable status commit truthfully and retains its received grant for a later probe")

        other = self.save("probe_during_login",url=self.url)
        authorization = self.cli("credentials","login",other)
        self.cli("connections","probe",other,exit_status=1)
        assert self.cli("connections","show",other)["connection"]["status"] == "needs_auth"
        assert urllib.request.urlopen(authorization["url"],timeout=15).status == 200
        assert not self.invoke(other,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        self.passed("a management probe while the browser login is pending cannot revoke the valid callback at the same revision")

        racing = self.save("stale_probe_callback",url=self.url)
        authorization = self.cli("credentials","login",racing)
        self.cli("connections","probe",racing,exit_status=1)
        previous = self.stats()["gates"].get("authorization_code",0)
        self.control("authorization_code","arm")
        with ThreadPoolExecutor(max_workers=2) as workers:
            callback = workers.submit(urllib.request.urlopen,authorization["url"],timeout=15)
            try:
                self.wait_gate("authorization_code",previous)
                probe = workers.submit(subprocess.run,[str(BINARY),"connections","probe",racing],
                    env=self.environment,capture_output=True,text=True,timeout=30)
                time.sleep(0.5)
            finally: self.control("authorization_code","release")
            response = callback.result(timeout=20)
            assert response.status == 200
            response.close()
            assert probe.result(timeout=30).returncode in [0,1]
        assert not self.invoke(racing,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        self.passed("a stale failed probe waiting behind the callback cannot overwrite a newly completed authorization")

    def contracts(self):
        source = self.save("contract_source",url=self.url)
        entry = self.cli("gateway","save","--stdin",value={"connectionIds":[source]})
        name = entry["members"][0]["namespace"]+"__read_document"
        description = self.cli("connections","tools",entry["id"],"--tool",name)
        direct = self.cli("connections","tools",source,"--tool","read_document")
        assert description["schemaHash"] != direct["schemaHash"]
        revision = description["tool"]["_meta"]["mcplink"]["upstreamRevision"]
        bridge = self.bridge(entry["id"],"contract-pins")
        self.invoke(entry["id"],name,{"id":"weekly-fixture"},schema_hash=description["schemaHash"])
        assert self.stats()["forwardedMcplinkMeta"] == 0
        before = self.stats()["calls"]
        self.cli("credentials","update",source,"--stdin",value={"value":"synthetic-fixture-authorization"})
        assert "error" in bridge.call("tools/call",{"name":name,"arguments":{"id":"weekly-fixture"},
            "_meta":{"mcplink":{"expectedUpstreamRevision":revision,"expectedSchemaHash":description["schemaHash"]}}})
        assert "error" in bridge.call("tools/call",{"name":name,"arguments":{"id":"weekly-fixture"},
            "_meta":{"mcplink":{"expectedSchemaHash":"0"*64}}})
        prompt = entry["members"][0]["namespace"]+"__weekly_report"
        assert "error" in bridge.call("prompts/get",{"name":prompt,
            "_meta":{"mcplink":{"expectedUpstreamRevision":revision}}})
        assert self.stats()["calls"] == before
        self.passed("public tool descriptors retain member revisions and alias-based hashes; external pins reject stale calls and internal metadata is stripped")

        changing = self.save("changing_contract",url=f"http://127.0.0.1:{self.port}/mcp-route-schema")
        changed_entry = self.cli("gateway","save","--stdin",value={"connectionIds":[changing]})
        alias = changed_entry["members"][0]["namespace"]+"__read_document"
        self.invoke(changed_entry["id"],alias,{"id":"weekly-fixture"},exit_status=1)
        assert self.stats()["calls"] == before
        self.passed("schema changes between aggregate discovery and selected-member dispatch refuse execution without retry")

    def login_interleave(self):
        connection = self.save("login_interleave",url=self.url,startupTimeout=5)
        authorization = self.cli("credentials","login",connection)
        assert urllib.request.urlopen(authorization["url"],timeout=15).status == 200
        metadata = self.stats()["gates"].get("metadata",0)
        refresh = self.stats()["gates"].get("refresh_token",0)
        self.control("metadata","arm")
        self.control("refresh_token","arm")
        with ThreadPoolExecutor(max_workers=2) as workers:
            login = workers.submit(self.cli,"credentials","login",connection)
            try:
                self.wait_gate("metadata",metadata)
                request = workers.submit(subprocess.run,[str(BINARY),"mcp","request",connection,"--stdin"],
                    input=json.dumps({"method":"tools/list"}),env=self.environment,capture_output=True,text=True,timeout=30)
                self.wait_gate("refresh_token",refresh)
            finally:
                self.control("metadata","release")
                self.control("refresh_token","release")
            login.result(timeout=30)
            response = request.result(timeout=30)
            assert response.returncode in [0,1] and not response.stderr
        self.passed("OAuth network preparation leaves the refresh guard available to a concurrently initializing shared session")

    def cleanup(self):
        connection = self.oauth("cleanup_retry")
        grant = self.data/"fixture-vault"/hashlib.sha256(("oauth/"+connection).encode()).hexdigest()
        old_grant = grant.read_bytes()
        grant.unlink()
        grant.mkdir()
        try:
            self.cli("credentials","oauth-config",connection,"--stdin",value={"scopes":["documents:write"]},exit_status=1)
        finally:
            grant.rmdir()
            grant.write_bytes(old_grant)
        self.cli("gateway","stop")
        self.cli("status")
        assert not grant.exists(), "restart lost the committed grant cleanup intent"
        authorization = self.cli("credentials","login",connection)
        assert urllib.request.urlopen(authorization["url"],timeout=15).status == 200
        self.cli("gateway","stop")
        assert not self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        assert grant.is_file(), "completed cleanup deleted a subsequent authorization"
        self.passed("restart retries committed grant cleanup exactly before a subsequent OAuth authorization")

        with sqlite3.connect(self.data/"mcplink.sqlite3") as db:
            definition = json.loads(db.execute("SELECT document FROM connections WHERE id=?",[connection]).fetchone()[0])["definitionRef"]
        file = self.data/"fixture-vault"/hashlib.sha256(definition.encode()).hexdigest()
        original = file.read_bytes()
        # The old definition is read before it can be cleaned; use a held token
        # exchange to replace it only after the preparation has read it.
        previous = self.stats()["gates"].get("metadata",0)
        self.control("metadata","arm")
        with ThreadPoolExecutor(max_workers=1) as workers:
            pending = workers.submit(self.cli,"credentials","login",connection,exit_status=1)
            try:
                self.wait_gate("metadata",previous)
                file.unlink()
                file.mkdir()
            finally: self.control("metadata","release")
            pending.result(timeout=30)
        file.rmdir()
        file.write_bytes(original)
        # Retry the same public definition, including metadata-only saves.
        current = self.cli("connections","show",connection)
        self.cli("connections","save","--stdin",value={"id":connection,"name":current["connection"]["name"],"definition":current["definition"]})
        assert not file.exists(), "metadata-only retry lost an old definition cleanup reference"
        self.passed("a same-definition retry completes old definition cleanup after a post-commit Vault failure")

    def definition_crash(self):
        connection = self.oauth("definition_crash")
        before = self.cli("connections","show",connection)
        vault = self.data/"fixture-vault"
        files = {file.name for file in vault.iterdir()}
        grant = vault/hashlib.sha256(("oauth/"+connection).encode()).hexdigest()
        saved_grant = grant.read_bytes()
        for point in ["definition-after-stage", "definition-after-commit"]:
            self.cli("gateway","stop")
            self.cli("credentials","oauth-config",connection,"--stdin",value={"scopes":["documents:write"]},
                exit_status=1,env=self.environment | {"MCPLINK_FAILPOINT":point})
            assert len(list(vault.iterdir())) == len(files)+1
            assert grant.read_bytes() == saved_grant
            current = self.cli("connections","show",connection)
            if point == "definition-after-stage":
                assert current == before and {file.name for file in vault.iterdir()} == files
                self.passed("a crash after staging cleans the orphan definition on restart and retains the original grant")
            else:
                assert current["definition"]["oauthScopes"] == ["documents:write"]
                assert current["connection"]["status"] == "needs_auth" and not grant.exists()
                self.passed("a crash after SQLite commit replays credential cleanup before accepting later traffic")

    def reconnect_cleanup(self):
        connection = self.oauth("reconnect_cleanup")
        grant = self.data/"fixture-vault"/hashlib.sha256(("oauth/"+connection).encode()).hexdigest()
        original = grant.read_bytes()
        grant.unlink()
        grant.mkdir()
        try:
            self.cli("credentials","disconnect",connection,exit_status=1)
        finally:
            grant.rmdir()
            grant.write_bytes(original)
        self.cli("rpc","connections.reconnect","--params",json.dumps({"id":connection}))
        assert not grant.exists(), "reconnect skipped the previous grant revocation cleanup"
        before = self.stats()["calls"]
        self.invoke(connection,"read_document",{"id":"weekly-fixture"},exit_status=1)
        assert self.stats()["calls"] == before
        self.passed("reconnect completes an interrupted disconnect cleanup before activating the connection")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record",type=Path,required=True)
    parser.add_argument("--case",choices=["catalog","persistence","refresh","callback","contracts","login_interleave","cleanup","definition_crash","reconnect_cleanup","all"],default="all")
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True,exist_ok=False)
    harness = ArchitectureBoundaries(record)
    conclusion, error = "PASS", None
    try:
        for case in ["catalog","persistence","refresh","callback","contracts","login_interleave","cleanup","definition_crash","reconnect_cleanup"] if args.case == "all" else [args.case]:
            getattr(harness,case)()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally: harness.close(False)
    (record/"report.json").write_text(json.dumps({"conclusion":conclusion,"error":error,
        "executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),
        "binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),
        "scenarios":harness.results,"commands":harness.commands},indent=2)+"\n")
    print(conclusion,record,flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__": main()
