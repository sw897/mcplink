"""Exercise preregistered OAuth clients, failure preservation and confidential refresh."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime,timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import socket
import sqlite3
import threading
import time
import traceback
import urllib.parse
import urllib.request
from http.server import BaseHTTPRequestHandler,ThreadingHTTPServer
from socketserver import TCPServer

from tool_calls import ToolCalls
from run import BINARY

SECRET="synthetic-registered-secret"
CLIENT="synthetic-registered-client"


class OAuthClients(ToolCalls):
    def __init__(self,record):
        super().__init__(record,fixture_args=["--registered-client","--token-auth","post"])

    def settings(self,connection,value,exit_status=0):
        return self.cli("credentials","oauth-config",connection,"--stdin",value=value,exit_status=exit_status)

    def upstream_state(self, **flags):
        url = self.url.removesuffix("/mcp") + "/control"
        request = urllib.request.Request(url, data=json.dumps({"action": "oauth-state", **flags}).encode(),
                                         headers={"Content-Type": "application/json"})
        with urllib.request.urlopen(request, timeout=5) as response:
            assert json.load(response) == {"ok": True}

    def cancellable_login(self):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        connection = self.save("cancellable_login", url=self.url)
        settings = {"clientId":CLIENT,"clientSecret":SECRET,"callbackPort":port}
        self.settings(connection, settings)
        with ThreadPoolExecutor(max_workers=3) as workers:
            metadata = self.stats()["gates"].get("metadata", 0)
            self.control("metadata", "arm")
            preparing = workers.submit(self.cli, "credentials", "login", connection, exit_status=1)
            try:
                self.wait_gate("metadata", metadata)
                unrelated = workers.submit(self.save, "unrelated_during_metadata", url=self.url).result(timeout=5)
                workers.submit(self.settings, connection, {"scopes":["documents:read"]}).result(timeout=5)
                assert "取消" in preparing.result(timeout=5)["error"]["message"]
            finally:
                self.control("metadata", "release")
            self.passed("an edit cancels a held metadata request while unrelated connection writes complete")

            first = self.cli("credentials", "login", connection)
            second = self.cli("credentials", "login", connection)
            assert first["url"] != second["url"]
            assert urllib.parse.parse_qs(urllib.parse.urlsplit(second["url"]).query)["redirect_uri"] == [f"http://127.0.0.1:{port}/callback"]
            exchange = self.stats()["gates"].get("authorization_code", 0)
            self.control("authorization_code", "arm")

            def browser_status(url):
                try:
                    with urllib.request.urlopen(url, timeout=12) as response:
                        return response.status
                except urllib.error.HTTPError as error:
                    error.close()
                    return error.code
                except OSError:
                    return None

            pending = workers.submit(browser_status, second["url"])
            try:
                self.wait_gate("authorization_code", exchange)
                workers.submit(self.settings, unrelated, {"callbackPort":port}).result(timeout=5)
                workers.submit(self.cli, "credentials", "disconnect", connection).result(timeout=5)
                assert pending.result(timeout=5) in {None, 400, 503}
            finally:
                self.control("authorization_code", "release")
        grant = self.data / "fixture-vault" / hashlib.sha256(f"oauth/{connection}".encode()).hexdigest()
        assert not grant.exists()
        assert self.cli("connections", "show", connection)["connection"]["status"] == "disconnected"
        self.passed("same-port retry replaces its callback; an interrupted exchange cannot commit after disconnect")

        self.cli("credentials", "login", connection)
        assert self.cli("gateway", "stop")["stopped"]
        # The next daemon owns recovery of the incomplete authorization.
        db = sqlite3.connect(self.data / "mcplink.sqlite3")
        try:
            row = db.execute("SELECT document FROM connections WHERE id=?", (connection,)).fetchone()
            assert json.loads(row[0])["status"] == "authorizing"
        finally:
            db.close()
        recovered = self.cli("connections", "show", connection)["connection"]
        assert recovered["status"] == "needs_auth" and "上次浏览器授权" in recovered["lastError"]
        authorization = self.cli("credentials", "login", connection)
        assert browser_status(authorization["url"]) == 200
        assert not self.invoke(connection, "read_document", {"id":"weekly-fixture"})["output"]["isError"]
        self.passed("daemon stop cancels a pending callback; the same configured port and connection authorize after restart")

    def run(self):
        offline=self.record/"offline"
        self.cli("--data-dir",str(offline),"credentials","oauth-config","missing","--stdin",raw="[]",exit_status=2)
        assert not offline.exists()
        self.passed("invalid OAuth configuration stdin fails offline without opening a backend")

        connection=self.save("registered_client",url=self.url)
        self.cli("connections","probe",connection)
        before=self.cli("connections","show",connection)
        failed=self.cli("credentials","login",connection,exit_status=1)
        assert "clientId" in failed["error"]["message"]
        after=self.cli("connections","show",connection)
        assert after["definition"]==before["definition"] and after["connection"]["status"]=="ready"
        assert self.stats()["registrations"]==0
        self.passed("a server without DCR reports missing registration while retaining an existing usable Token and status")

        for value in [{"clientSecret":SECRET},{"callbackPort":0},{"callbackPort":65536},{"scopes":["has whitespace"]},{"scopes":[1]},{"unknown":SECRET}]:
            failure=self.settings(connection,value,exit_status=1)
            assert SECRET not in json.dumps(failure)
            assert self.cli("connections","show",connection)["definition"]==before["definition"]
        configured=self.settings(connection,{"clientId":CLIENT,"clientSecret":SECRET,"scopes":["documents:read"],"callbackPort":self.port})
        definition=configured["definition"]
        assert definition["clientSecret"]=="••••••••" and not definition["oauth"]
        assert definition["headers"]["Authorization"]=="••••••••"
        assert SECRET not in json.dumps(configured)
        self.passed("configuration validates atomically, masks client secrets and retains the current authentication mode")

        failed=self.cli("credentials","login",connection,exit_status=1)
        assert "回调" in failed["error"]["message"]
        assert self.cli("connections","show",connection)["definition"]==definition
        with socket.socket() as listener:
            listener.bind(("127.0.0.1",0));port=listener.getsockname()[1]
        self.settings(connection,{"callbackPort":port})
        authorization=self.cli("credentials","login",connection)
        query=urllib.parse.parse_qs(urllib.parse.urlsplit(authorization["url"]).query)
        assert query["redirect_uri"]==[f"http://127.0.0.1:{port}/callback"]
        assert query["client_id"]==[CLIENT] and query["code_challenge_method"]==["S256"]
        assert SECRET not in authorization["url"]
        assert set(query["scope"][0].split())=={"documents:read","offline_access"}
        assert urllib.request.urlopen(authorization["url"],timeout=15).status==200
        assert self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"] is False
        assert self.stats()["registrations"]==0 and self.stats()["tokenAuthChecks"]>=1
        self.passed("a busy fixed callback preserves prior auth; preregistered PKCE login uses explicit scopes and client_secret_post without DCR")

        unchanged=self.settings(connection,{"clientSecret":"••••••••","scopes":["documents:read"],"callbackPort":port})
        assert unchanged["status"]=="unchanged"
        self.cli("connections","probe",connection)
        self.cli("changes","preview","--connection",connection,"--target","hermes:user",exit_status=1)
        plan=self.cli("changes","preview","--connection",connection,"--target","codex:user","--target","hermes:user","--mode","shared")
        assert all(row["status"]=="applied" for row in self.cli("changes","apply",plan["planId"])["results"])
        generated=self.cli("cli","generate",connection,"--name","oauth-tools","--output",str(self.record/"generated"),"--tool","read_document")
        for path in [self.home/".codex/config.toml",self.home/".hermes/config.yaml",Path(generated["manifest"]),*self.data.glob("*.db*"),*self.data.glob("*.sqlite3*")]:
            assert SECRET.encode() not in path.read_bytes(),path
        assert SECRET not in json.dumps(self.cli("status"))
        self.passed("masked edits preserve grants; shared/native references, SQLite, management output and generated files omit client secrets")

        clients=[self.bridge(connection,"confidential-"+str(index)) for index in range(2)]
        for client in clients: assert "result" in client.call("tools/list")
        before_calls=self.stats()
        time.sleep(4)
        def bridge_call(client): return client.call("tools/call",{"name":"read_document","arguments":{"id":"weekly-fixture"}})
        with ThreadPoolExecutor(max_workers=3) as workers:
            futures=[workers.submit(bridge_call,client) for client in clients]
            futures.append(workers.submit(self.invoke,connection,"read_document",{"id":"weekly-fixture"}))
            values=[future.result(timeout=30) for future in futures]
        assert all("result" in value for value in values[:2]) and not values[2]["output"]["isError"], {"bridge_errors":[value.get("error") for value in values[:2]],"stats":self.stats()}
        # The fixture's three-second grants stay inside the SDK's proactive
        # refresh window, so each request may refresh. Rotation must serialize
        # and reload the latest grant rather than reuse a stale refresh token.
        after_calls=self.stats()
        assert after_calls["refreshes"]>before_calls["refreshes"]
        assert after_calls["calls"]==before_calls["calls"]+len(futures)
        self.passed("two established Bridges and CLI serialize confidential refresh and reuse the rotated grant")

        runtime=json.loads((self.data/"runtime.json").read_text());os.kill(runtime["pid"],signal.SIGTERM);time.sleep(0.4)
        time.sleep(4);refreshes=self.stats()["refreshes"]
        assert self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"] is False
        assert self.stats()["refreshes"]>refreshes
        for client in clients: assert "result" in bridge_call(client)
        assert self.cli("cli","run","--manifest",generated["manifest"],"--","read_document","--stdin",value={"id":"weekly-fixture"})["output"]["isError"] is False
        self.passed("backend restart restores the confidential client secret for refresh and keeps existing Bridge/generated references usable")

        grant = self.data / "fixture-vault" / hashlib.sha256(f"oauth/{connection}".encode()).hexdigest()
        self.cli("gateway", "stop")
        saved = grant.read_bytes()
        self.upstream_state(metadataUnavailable=True)
        try:
            failure = self.cli("connections", "probe", connection, exit_status=1)
            assert failure["result"]["connection"]["status"] == "error", failure
            assert grant.read_bytes() == saved
        finally:
            self.upstream_state()
        assert "result" in bridge_call(clients[0])
        assert not self.cli("cli", "run", "--manifest", generated["manifest"], "--", "read_document", "--stdin", value={"id": "weekly-fixture"})["output"]["isError"]
        self.passed("metadata 503 keeps the grant and existing Bridge and generated CLI recover without another login")

        self.cli("gateway", "stop")
        with sqlite3.connect(self.data / "mcplink.sqlite3") as database:
            raw, = database.execute("SELECT document FROM connections WHERE id=?", (connection,)).fetchone()
            previous = json.loads(raw)
            database.execute("UPDATE connections SET document=? WHERE id=?",
                             (json.dumps(previous | {"status": "needs_auth"}), connection))
        self.cli("connections", "tools", connection, exit_status=1)
        verified = self.cli("connections", "probe", connection)["connection"]
        assert verified["status"] == "ready" and verified["revision"] == previous["revision"]
        self.passed("explicit recheck recovers a legacy needs_auth row with a retained grant")

        self.cli("gateway", "stop")
        self.upstream_state(rejectBearer=True)
        try:
            failed = self.cli("connections", "probe", connection, exit_status=1)
            assert failed["result"]["connection"]["status"] == "needs_auth"
        finally:
            self.upstream_state()
        assert self.cli("connections", "probe", connection)["connection"]["status"] == "ready"
        self.cli("gateway", "stop")
        self.upstream_state(revokeGrant=True)
        failed = self.cli("connections", "probe", connection, exit_status=1)
        assert failed["result"]["connection"]["status"] == "needs_auth" and not grant.exists()
        calls_before_recheck = self.stats()["calls"]
        self.cli("connections", "probe", connection, exit_status=1)
        assert self.stats()["calls"] == calls_before_recheck
        self.passed("real upstream 401 and rejected refresh tokens require authentication; a revoked grant is discarded")

        calls=self.stats()["calls"]
        self.settings(connection,{"scopes":["offline_access"]})
        assert self.cli("connections","show",connection)["connection"]["status"]=="needs_auth"
        self.invoke(connection,"read_document",{"id":"weekly-fixture"},exit_status=1)
        assert self.stats()["calls"]==calls
        cleared=self.settings(connection,{"clientSecret":None,"clientId":None,"scopes":[],"callbackPort":None})
        assert cleared["definition"]["clientSecret"] is None and cleared["definition"]["clientId"] is None
        self.passed("changing application scopes invalidates prior authorization before dispatch; explicit null clears stored registration")

        basic_record=self.record/"basic";basic_record.mkdir()
        basic=ToolCalls(basic_record,fixture_args=["--registered-client","--token-auth","basic"])
        try:
            other=basic.save("basic_client",url=basic.url)
            basic.cli("credentials","oauth-config",other,"--stdin",value={"clientId":CLIENT,"clientSecret":SECRET,"scopes":["documents:read"]})
            authorization=basic.cli("credentials","login",other)
            assert urllib.request.urlopen(authorization["url"],timeout=15).status==200
            assert basic.invoke(other,"read_document",{"id":"weekly-fixture"})["output"]["isError"] is False
            time.sleep(4)
            assert basic.invoke(other,"read_document",{"id":"weekly-fixture"})["output"]["isError"] is False
            assert basic.stats()["refreshes"]>=1 and basic.stats()["registrations"]==0
        finally: basic.close(False)
        self.passed("client_secret_basic authorization and refresh also follow discovered server metadata")

        self.cancellable_login()

        runtime_path=self.data/"runtime.json";runtime_bytes=runtime_path.read_bytes();runtime=json.loads(runtime_bytes);posts=[]
        class Legacy(BaseHTTPRequestHandler):
            def log_message(self,*_): pass
            def do_GET(self):
                data=json.dumps({"application":"mcplink","instance":runtime["instance"],"managementApi":3}).encode()
                self.send_response(200);self.send_header("Content-Length",str(len(data)));self.end_headers();self.wfile.write(data)
            def do_POST(self): posts.append(True);self.send_response(500);self.end_headers()
        class LegacyServer(ThreadingHTTPServer):
            def server_bind(self):
                TCPServer.server_bind(self);self.server_name,self.server_port=self.server_address
        legacy=LegacyServer(("127.0.0.1",0),Legacy);thread=threading.Thread(target=legacy.serve_forever,daemon=True);thread.start()
        try:
            runtime_path.write_text(json.dumps(runtime|{"port":legacy.server_port}))
            for arguments,value in [(("credentials","oauth-config",connection,"--stdin"),{"clientId":CLIENT,"clientSecret":SECRET}),(("credentials","login",connection),None)]:
                failure=self.cli(*arguments,value=value,exit_status=1)
                assert "旧后台" in failure["error"]["message"] and SECRET not in json.dumps(failure)
            assert not posts
        finally: runtime_path.write_bytes(runtime_bytes);legacy.shutdown();legacy.server_close();thread.join()
        self.passed("old backends reject new configuration and login semantics before secrets or unsupported RPCs are sent")
        return {"connection":connection}


def main():
    parser=argparse.ArgumentParser();parser.add_argument("--record",type=Path,required=True)
    record=parser.parse_args().record.resolve();record.mkdir(parents=True,exist_ok=False)
    harness=OAuthClients(record);conclusion,error,ids="PASS",None,{}
    try: ids=harness.run()
    except Exception as exception: conclusion,error="FAIL",str(exception);traceback.print_exc()
    finally: harness.close(False)
    report={"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands,"ids":ids,"real_native_mcp_config_modified":False}
    (record/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(conclusion,record,flush=True);raise SystemExit(conclusion!="PASS")


if __name__=="__main__": main()
