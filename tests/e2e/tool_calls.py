"""Exercise tool discovery, validation and invocation through the public CLI and Bridge."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import time
import traceback
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from socketserver import TCPServer
import threading

from agent_cli import AgentCli
from run import BINARY, ROOT


class ToolCalls(AgentCli):
    def stats(self):
        with urllib.request.urlopen(f"http://127.0.0.1:{self.port}/stats",timeout=5) as response:
            return json.load(response)

    def save(self, name, **definition):
        base={"url":f"http://127.0.0.1:{self.port}/mcp-tools","headers":{"Authorization":"synthetic-fixture-authorization"}}
        return self.cli("connections","save","--stdin",value={"name":name,"definition":base|definition})["id"]

    def invoke(self, connection, tool, value, *, exit_status=0, schema_hash=None):
        args=["call",connection,tool,"--stdin"]
        if schema_hash is not None: args += ["--schema-hash",schema_hash]
        return self.cli(*args,value=value,exit_status=exit_status)

    def run(self):
        offline=self.record/"offline"
        schema=self.cli("--data-dir",str(offline),"schema")
        assert any(c["name"]=="call" for c in schema["command"]["commands"]), "tool call command is absent"
        for args in [("call","missing","read_document"),("call","missing"),("call",)]:
            self.cli("--data-dir",str(offline),*args,exit_status=2)
        for raw in ['[]','{"value":',' '*(8*1024*1024+1)]:
            self.cli("--data-dir",str(offline),"call","missing","read_document","--stdin",raw=raw,exit_status=2)
        help_result=subprocess.run([str(BINARY),"call","--help"],env=self.environment,capture_output=True,text=True,timeout=10)
        assert help_result.returncode==0 and "--stdin" in help_result.stdout
        assert not offline.exists(), "invalid/offline commands started a daemon"
        self.passed("offline discovery and argument/stdin rejection do not open a Vault or daemon")

        connection=self.save("typed_tools")
        listed=self.cli("connections","tools",connection)
        assert len(listed["tools"])==5 and listed["connection"]["toolCount"]==5
        description=self.cli("connections","tools",connection,"--tool","inspect_payload")
        assert description["tool"]["name"]=="inspect_payload" and len(description["schemaHash"])==64
        assert description["validation"]["externalReferences"] is False
        self.passed("exact tool discovery follows pages and reports the schema, description and contract digest")

        before=self.stats()["calls"]
        payload={"items":[{"label":"fixture","weight":1.5}],"enabled":False,"count":2,"choice":"a","optional":None}
        result=self.invoke(connection,"inspect_payload",payload,schema_hash=description["schemaHash"])
        assert result["connectionId"]==connection and result["tool"]=="inspect_payload"
        actual=json.loads(result["output"]["content"][0]["text"])
        assert actual["received"]==payload
        assert self.stats()["calls"]==before+1
        self.passed("typed nested objects, arrays, booleans, numbers and null reach the selected tool unchanged")

        empty=json.dumps({"query":""})
        large={"query":"x"*(8*1024*1024-len(empty.encode()))}
        assert len(json.dumps(large).encode())==8*1024*1024
        assert self.invoke(connection,"search_documents",large)["output"]["isError"] is False
        self.passed("an input at the full 8 MiB limit survives the local RPC envelope")

        before=self.stats()["calls"]
        sentinel="synthetic-invalid-secret-never-echo"
        invalid=[{},payload|{"enabled":sentinel},payload|{"count":0},payload|{"count":1.5},payload|{"choice":sentinel},payload|{"extra":sentinel},payload|{"items":[{"label":"x","weight":-1}]},payload|{"items":[]}]
        for value in invalid:
            failure=self.invoke(connection,"inspect_payload",value,exit_status=2)
            assert failure["error"]["code"]=="invalid_tool_arguments"
            assert sentinel not in json.dumps(failure)
            assert failure["result"]["issues"]
        assert self.stats()["calls"]==before
        self.passed("required, type, enum, bounds, additional properties, arrays and local refs fail before dispatch without input echo")

        for tool in ["invalid_schema","external_schema"]:
            failure=self.invoke(connection,tool,{"value":sentinel},exit_status=1)
            assert failure["error"]["code"]=="invalid_tool_schema"
            assert sentinel not in json.dumps(failure)
        assert self.stats()["calls"]==before
        assert self.stats()["schemaReads"]==0, "validation fetched a remote schema"
        self.passed("invalid schemas and external references refuse execution instead of skipping validation")

        failure=self.invoke(connection,"inspect_payload",payload,schema_hash="0"*64,exit_status=1)
        assert failure["error"]["code"]=="tool_schema_changed"
        for tool in ["missing",sentinel]:
            failure=self.invoke(connection,tool,{},exit_status=1)
            assert failure["error"]["code"]=="tool_not_found" and sentinel not in json.dumps(failure)
        filtered=self.save("filtered_tools",includeTools=["read_document"])
        failure=self.invoke(filtered,"inspect_payload",payload,exit_status=1)
        assert failure["error"]["code"]=="tool_not_found"
        assert self.stats()["calls"]==before
        self.passed("missing/filtered tools and changed pinned schemas never dispatch or choose a different tool")

        for endpoint in ["mcp-cycle","mcp-duplicates"]:
            invalid=self.save(endpoint.replace("-","_"),url=f"http://127.0.0.1:{self.port}/{endpoint}")
            failure=self.invoke(invalid,"read_document",{"id":"weekly-fixture"},exit_status=1)
            assert failure["error"]["code"]=="tool_discovery_failed"
        assert self.stats()["calls"]==before
        self.passed("cyclic pagination and duplicate tool names fail closed without tool execution")

        bridge=self.bridge(connection,"tool-calls-fixture")
        native=bridge.call("tools/call",{"name":"inspect_payload","arguments":payload})
        assert native["result"]==result["output"]
        self.passed("the public CLI and native MCP Bridge return the same upstream result")

        failed=self.invoke(connection,"read_document",{"id":"missing"},exit_status=1)
        assert failed["error"]["code"]=="upstream_failed" and failed["result"]["output"]["isError"]
        rpc_error=self.save("rpc_error_tools",url=f"http://127.0.0.1:{self.port}/mcp-rpc-error")
        failed=self.invoke(rpc_error,"search_documents",{"query":"fixture"},exit_status=1)
        assert failed["error"]["code"]=="upstream_failed" and "rpcError" in failed["result"]["output"]
        self.passed("upstream tool errors preserve their MCP result and return exit one")

        slow=self.save("slow_tools",url=f"http://127.0.0.1:{self.port}/mcp-slow")
        before=self.stats()
        with ThreadPoolExecutor(max_workers=1) as workers:
            pending=workers.submit(self.invoke,slow,"inspect_payload",payload,exit_status=1)
            deadline=time.monotonic()+20
            while self.stats()["lists"]==before["lists"]:
                assert time.monotonic()<deadline, "discovery did not start"
                time.sleep(0.03)
            self.cli("credentials","update",slow,"--stdin",value={"value":"synthetic-fixture-authorization"})
            pending.result(timeout=30)
        assert self.stats()["calls"]==before["calls"]
        self.passed("connection mutation during discovery never dispatches with a stale validation context")

        self.cli("gateway","pause")
        paused=self.invoke(connection,"inspect_payload",payload,exit_status=1)
        assert paused["error"]["code"]=="operation_failed"
        self.cli("gateway","resume")
        self.cli("credentials","disconnect",connection)
        self.invoke(connection,"inspect_payload",payload,exit_status=1)
        self.cli("credentials","update",connection,"--stdin",value={"value":"synthetic-fixture-authorization"})
        assert self.invoke(connection,"inspect_payload",payload)["output"]["isError"] is False
        self.invoke("missing","inspect_payload",payload,exit_status=1)
        self.invoke("typed_tools","inspect_payload",payload,exit_status=1)
        self.passed("paused, disconnected and missing connections fail and managed credential updates recover calls")

        runtime_path=self.data/"runtime.json"
        runtime_bytes=runtime_path.read_bytes()
        runtime=json.loads(runtime_bytes)
        posts=[]
        class Legacy(BaseHTTPRequestHandler):
            def log_message(self,*_): pass
            def do_GET(self):
                data=json.dumps({"application":"mcplink","instance":runtime["instance"],"version":"0.1.0"}).encode()
                self.send_response(200);self.send_header("Content-Length",str(len(data)));self.end_headers();self.wfile.write(data)
            def do_POST(self):
                posts.append(True)
                self.send_response(500);self.end_headers()
        class LegacyServer(ThreadingHTTPServer):
            def server_bind(self):
                TCPServer.server_bind(self)
                self.server_name,self.server_port=self.server_address
        legacy=LegacyServer(("127.0.0.1",0),Legacy)
        thread=threading.Thread(target=legacy.serve_forever,daemon=True);thread.start()
        try:
            runtime_path.write_text(json.dumps(runtime|{"port":legacy.server_port}))
            failure=self.invoke(connection,"inspect_payload",payload,exit_status=1)
            assert "旧后台" in failure["error"]["message"] and not posts
        finally:
            runtime_path.write_bytes(runtime_bytes);legacy.shutdown();legacy.server_close();thread.join()
        self.passed("a running legacy daemon is detected before unsupported RPCs are sent")

        oauth=self.cli("connections","save","--stdin",value={"name":"oauth_tools","definition":{"url":self.url,"oauth":True}})["id"]
        authorization=self.cli("credentials","login",oauth)
        assert urllib.request.urlopen(authorization["url"],timeout=15).status==200
        assert self.invoke(oauth,"read_document",{"id":"weekly-fixture"})["output"]["isError"] is False
        refreshes=self.stats()["refreshes"]
        time.sleep(4)
        assert self.invoke(oauth,"read_document",{"id":"weekly-fixture"})["output"]["isError"] is False
        assert self.stats()["refreshes"]>refreshes
        self.passed("named tool invocation reuses managed OAuth, persisted grants and token refresh")

        timed=self.save("timed_tools",timeout=5)
        count=self.stats()["calls"]
        failure=self.invoke(timed,"read_document",{"id":"weekly-fixture","delay_ms":6000},exit_status=1)
        assert "结果" in failure["error"]["message"]
        time.sleep(1.3)
        assert self.stats()["calls"]==count+1
        self.passed("an uncertain timeout sends the tool exactly once and is not automatically replayed")

        dropped=self.save("dropped_tools",url=f"http://127.0.0.1:{self.port}/mcp-drop")
        count=self.stats()["calls"]
        failure=self.invoke(dropped,"search_documents",{"query":"fixture"},exit_status=1)
        assert "结果" in failure["error"]["message"]
        assert self.stats()["calls"]==count+1
        self.passed("a connection closed after receipt is reported as uncertain and the tool is not resent")
        return {"connection":connection}


def main():
    parser=argparse.ArgumentParser();parser.add_argument("--record",type=Path,required=True)
    record=parser.parse_args().record.resolve();record.mkdir(parents=True,exist_ok=False)
    harness=ToolCalls(record);conclusion,error,ids="PASS",None,{}
    try: ids=harness.run()
    except Exception as exception:
        conclusion,error="FAIL",str(exception);traceback.print_exc()
    finally: harness.close(False)
    report={"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands,"ids":ids,"real_native_mcp_config_modified":False}
    (record/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(conclusion,record,flush=True)
    raise SystemExit(conclusion!="PASS")


if __name__=="__main__": main()
