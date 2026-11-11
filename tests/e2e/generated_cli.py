"""Run the actual generated POSIX/Windows entry points against isolated managed MCP."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import signal
import subprocess
import time
import traceback
import threading
import urllib.request
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from socketserver import TCPServer

from tool_calls import ToolCalls
from run import BINARY, ROOT


class GeneratedCli(ToolCalls):
    def __init__(self, record):
        super().__init__(record)
        self.extra_data = []
        self.generated = []
        runtime = record / "runtime with spaces"
        runtime.mkdir()
        self.runtime = runtime / BINARY.name
        shutil.copy2(BINARY, self.runtime)
        self.generated_env = self.environment | {"MCPLINK_BIN":str(self.runtime)}

    def generate(self, connection, directory, name="fixture-tools", tools=None):
        tools = tools or ["inspect_payload","read_document","search_documents"]
        result = self.cli("cli","generate",connection,"--name",name,"--output",str(directory),
                          *[part for tool in tools for part in ["--tool",tool]])
        self.generated.append(directory)
        return Path(result["manifest"])

    def launch(self, manifest, *args, value=None, raw=None, exit_status=0, help_text=False, env=None):
        name=json.loads(manifest.read_text())["name"]
        executable=manifest.parent/(name+(".ps1" if os.name=="nt" else ""))
        command=["powershell.exe","-NoProfile","-ExecutionPolicy","Bypass","-File",str(executable)] if os.name=="nt" else [str(executable)]
        process=subprocess.Popen([*command,*args],stdin=subprocess.PIPE,stdout=subprocess.PIPE,stderr=subprocess.PIPE,
                                 env=env or self.generated_env,text=True)
        try:
            stdout,stderr=process.communicate(json.dumps(value) if value is not None else raw or "",timeout=30)
        except subprocess.TimeoutExpired:
            # An inherited pipe can keep communicate waiting after its parent exits.
            # Stop only daemons belonging to this synthetic fixture before draining it.
            for data in [self.data,*self.extra_data]:
                try: os.kill(json.loads((data/"runtime.json").read_text())["pid"],signal.SIGTERM)
                except (FileNotFoundError,ProcessLookupError): pass
            process.kill();process.communicate(timeout=10)
            raise AssertionError("Generated launcher exceeded 30 seconds; fixture daemons stopped")
        result=subprocess.CompletedProcess(command,process.returncode,stdout,stderr)
        self.commands.append({"entry":"generated PowerShell" if os.name=="nt" else "generated POSIX", "arguments_sha256":hashlib.sha256(json.dumps(args).encode()).hexdigest(),"exit_status":result.returncode,"stdout_sha256":hashlib.sha256(result.stdout.encode()).hexdigest()})
        assert result.returncode==exit_status,(args,result.returncode,result.stdout,result.stderr)
        assert not result.stderr,(args,result.stderr)
        if help_text: return result.stdout
        response=json.loads(result.stdout)
        if exit_status==0:
            assert "ok" in response,response
            return response["ok"]
        assert "error" in response,response
        return response

    def definition(self, connection, **updates):
        detail=self.cli("connections","show",connection)
        self.cli("connections","save","--stdin",value={"id":connection,"name":detail["connection"]["name"],"definition":detail["definition"]|updates})

    def run(self):
        offline=self.record/"offline-data"
        schema=self.cli("--data-dir",str(offline),"schema")
        assert any(command["name"]=="cli" for command in schema["command"]["commands"])
        for name in ["../bad","mcplink","CON","com1","a.b","bad name"]:
            self.cli("--data-dir",str(offline),"cli","generate","missing","--name",name,"--output",str(self.record/"invalid"),"--tool","read_document",exit_status=2)
        for tools in [[],["help"],["bad/name"],["read_document","read_document"]]:
            self.cli("--data-dir",str(offline),"cli","generate","missing","--name","fixture-tools","--output",str(self.record/"invalid"),*[part for tool in tools for part in ["--tool",tool]],exit_status=2)
        assert not offline.exists()
        self.passed("invalid names, selections and collisions fail offline before opening a backend")

        connection=self.save("generated_tools",url=f"http://127.0.0.1:{self.port}/mcp-generated")
        manifest=self.generate(connection,self.record/"生成入口 with spaces")
        inspected=self.cli("--data-dir",str(offline),"cli","inspect",str(manifest))
        assert inspected["connectionId"]==connection and len(inspected["tools"])==3
        assert self.launch(manifest,"--help",help_text=True).count("inspect_payload")==1
        help_result=self.launch(manifest,"inspect_payload","--help",help_text=True,env=self.generated_env|{"MCPLINK_DATA_DIR":str(offline)})
        assert "--enabled <BOOLEAN>" in help_result and "--items <JSON>" in help_result
        assert not offline.exists()
        self.passed("generated help and inspect are offline, with explicit types and only selected tool names")

        payload={"items":[{"label":"空格 Unicode & echo synthetic-shell-argument ! % ^ 测试","weight":1.5}],"enabled":False,"count":2,"choice":"a","optional":None}
        flags=["--items",json.dumps(payload["items"],ensure_ascii=False),"--enabled","false","--count","2","--choice",'"a"',"--optional","null"]
        typed=self.launch(manifest,"inspect_payload",*flags)
        streamed=self.launch(manifest,"inspect_payload","--stdin",value=payload)
        generic=self.invoke(connection,"inspect_payload",payload)
        bridge=self.bridge(connection,"generated-cli")
        native=bridge.call("tools/call",{"name":"inspect_payload","arguments":payload})
        assert typed==streamed==generic and native["result"]==typed["output"]
        assert json.loads(typed["output"]["content"][0]["text"])["received"]==payload
        assert self.launch(manifest,"search_documents","--query","")["output"]["isError"] is False
        assert self.launch(manifest,"read_document","--id","weekly-fixture")["output"]["isError"] is False
        self.passed("actual launcher preserves nested JSON, boolean, number, null, Unicode, empty strings and paths with spaces; matches CLI and Bridge")

        before=self.stats()["calls"]
        sentinel="synthetic-invalid-secret-generated"
        for args in [("inspect_payload",*flags,"--count","3"),("inspect_payload",*flags,"--stdin"),
                     ("inspect_payload",*flags,"--unknown",sentinel),("read_document","--delay_ms",sentinel),
                     ("search_documents","--query","ok","--stdin"),("missing",sentinel)]:
            failed=self.launch(manifest,*args,exit_status=2)
            assert sentinel not in json.dumps(failed)
        self.launch(manifest,"inspect_payload","--stdin",raw="[]",exit_status=2)
        invalid=self.launch(manifest,"inspect_payload","--stdin",value=payload|{"count":0},exit_status=2)
        assert invalid["error"]["code"]=="invalid_tool_arguments"
        assert self.stats()["calls"]==before
        self.passed("duplicate, unknown, conflicting, missing, mistyped and invalid schema arguments never dispatch or echo values")

        odd=self.generate(connection,self.record/"odd-tools",name="odd-tools",tools=["odd_parameters","schema_secrets"])
        odd_json=self.cli("cli","inspect",str(odd))
        assert odd_json["tools"][0]["stdinOnly"] is True
        assert self.launch(odd,"odd_parameters","--stdin",value={"stdin":"fixture","odd/name":False})["output"]["isError"] is False
        self.launch(odd,"odd_parameters","--stdin","--help",help_text=True)
        for directory in self.generated:
            files=list(directory.iterdir());assert len(files)==3
            joined=b"\n".join(file.read_bytes() for file in files)
            for forbidden in [b"synthetic-fixture-authorization",b"synthetic-schema-default-private",b"synthetic-schema-example-private",b"synthetic-schema-enum-private",b"synthetic-schema-description-private",str(ROOT).encode(),str(self.home).encode(),b"127.0.0.1",b"inputSchema"]:
                assert forbidden not in joined,(directory,forbidden)
            assert sum(file.stat().st_size for file in files)<20000
        self.passed("unsafe property names use stdin; generated files omit full schemas, defaults, examples, endpoint, credentials and absolute paths")

        existing=self.record/"existing";existing.mkdir(); marker=existing/"keep.txt";marker.write_text(sentinel)
        self.cli("cli","generate",connection,"--name","preserve","--output",str(existing),"--tool","read_document",exit_status=1)
        assert marker.read_text()==sentinel and len(list(existing.iterdir()))==1
        linked=self.record/"linked-output";linked.symlink_to(existing,target_is_directory=True)
        self.cli("cli","generate",connection,"--name","preserve","--output",str(linked),"--tool","read_document",exit_status=1)
        for name in ["missing","invalid_schema","external_schema"]:
            output=self.record/f"bad-schema-{name}"
            self.cli("cli","generate",connection,"--name","bad-tools","--output",str(output),"--tool",name,exit_status=1)
            assert not output.exists()
        concurrent=self.record/"concurrent-generate"
        def generate_race():
            return subprocess.run([str(BINARY),"cli","generate",connection,"--name","race-tools","--output",str(concurrent),"--tool","read_document"],env=self.environment,capture_output=True,text=True,timeout=90)
        with ThreadPoolExecutor(max_workers=2) as workers:
            races=list(workers.map(lambda _:generate_race(),range(2)))
        assert sorted(result.returncode for result in races)==[0,1]
        assert len(list(concurrent.iterdir()))==3
        self.cli("cli","inspect",str(concurrent/"race-tools.mcplink.json"))
        self.passed("existing files, linked destinations, missing/invalid tools and concurrent generation cannot overwrite output")

        interrupted=self.record/"interrupted-generation"
        # Generation now runs in the managed backend; inject the failure at its
        # process boundary, with no live Bridge able to race daemon startup.
        for client in self.clients:
            client.close()
        self.clients.clear()
        self.cli("gateway","stop")
        self.cli("cli","generate",connection,"--name","retry-tools","--output",str(interrupted),"--tool","read_document",env=self.environment|{"MCPLINK_FAILPOINT":"cli-after-launchers"},exit_status=1)
        assert not interrupted.exists()
        self.cli("gateway","stop")
        self.generate(connection,interrupted,name="retry-tools",tools=["read_document"])
        self.passed("an interrupted fixture write removes only newly created files and permits a clean retry")

        bad=self.record/"bad-manifest.json"
        original=manifest.read_bytes()
        for raw in [b"{",b" "*(1024*1024+1),json.dumps(inspected|{"manifestVersion":99}).encode(),json.dumps(inspected|{"unknown":"synthetic-private-value"}).encode()]:
            bad.write_bytes(raw)
            failed=self.cli("--data-dir",str(offline),"cli","inspect",str(bad),exit_status=1)
            assert "synthetic-private-value" not in json.dumps(failed)
        link_manifest=self.record/"linked-manifest.json";link_manifest.symlink_to(manifest)
        self.cli("--data-dir",str(offline),"cli","inspect",str(link_manifest),exit_status=1)
        assert not offline.exists()
        self.passed("damaged, oversized, unsupported, unknown-field and symlink manifests fail offline without exposing contents")

        self.definition(connection,url=f"http://127.0.0.1:{self.port}/mcp-generated-changed")
        before=self.stats()["calls"]
        changed=self.launch(manifest,"read_document","--id","weekly-fixture",exit_status=1)
        assert changed["error"]["code"]=="tool_schema_changed" and self.stats()["calls"]==before
        self.definition(connection,url=f"http://127.0.0.1:{self.port}/mcp-generated",includeTools=["read_document"])
        filtered=self.launch(manifest,"inspect_payload","--stdin",value=payload,exit_status=1)
        assert filtered["error"]["code"]=="tool_not_found" and self.stats()["calls"]==before
        self.definition(connection,includeTools=[],includeSpecified=False)
        self.cli("credentials","update",connection,"--stdin",value={"value":"synthetic-fixture-authorization"})
        assert manifest.read_bytes()==original
        assert self.launch(manifest,"read_document","--id","weekly-fixture")["output"]["isError"] is False
        self.passed("changed schemas and current filters refuse dispatch; credential updates preserve existing generated files")

        foreign=self.record/"foreign 数据 with spaces";self.extra_data.append(foreign)
        foreign_env=self.generated_env|{"MCPLINK_DATA_DIR":str(foreign)}
        before=self.stats()["calls"]
        failed=self.launch(manifest,"read_document","--id","weekly-fixture",env=foreign_env,exit_status=1)
        assert "rebind" in failed["error"]["message"] and self.stats()["calls"]==before
        other=self.cli("connections","save","--stdin",value={"name":"generated_tools","definition":{"url":f"http://127.0.0.1:{self.port}/mcp-generated","headers":{"Authorization":"synthetic-fixture-authorization"}}},env=foreign_env)["id"]
        self.cli("cli","rebind",str(manifest),"--connection",other,env=foreign_env)
        assert self.launch(manifest,"read_document","--id","weekly-fixture",env=foreign_env)["output"]["isError"] is False
        self.launch(manifest,"read_document","--id","weekly-fixture",exit_status=1)
        self.cli("cli","rebind",str(manifest),"--connection",connection)
        assert self.launch(manifest,"read_document","--id","weekly-fixture")["output"]["isError"] is False
        lock=manifest.with_name("."+manifest.name+".lock")
        assert lock.is_file() and lock.stat().st_size==0
        self.passed("another dataset requires explicit rebind, validates every selected contract and never falls back by display name")

        slow=self.save("slow_rebind",url=f"http://127.0.0.1:{self.port}/mcp-slow")
        before=self.stats()["lists"];original=manifest.read_bytes()
        with ThreadPoolExecutor(max_workers=1) as workers:
            pending=workers.submit(self.cli,"cli","rebind",str(manifest),"--connection",slow,exit_status=1)
            deadline=time.monotonic()+20
            while self.stats()["lists"]==before:
                assert time.monotonic()<deadline
                time.sleep(0.03)
            locked=self.cli("cli","rebind",str(manifest),"--connection",connection,exit_status=1)
            assert "其他进程" in locked["error"]["message"]
            manifest.write_bytes(original+b"\n ")
            assert "并发" in pending.result(timeout=30)["error"]["message"]
        assert manifest.read_bytes()==original+b"\n "
        self.definition(slow,url=f"http://127.0.0.1:{self.port}/mcp-generated-changed")
        self.cli("cli","rebind",str(manifest),"--connection",slow,exit_status=1)
        assert manifest.read_bytes()==original+b"\n "
        lock.write_text("preserve unknown lock content")
        self.cli("cli","rebind",str(manifest),"--connection",connection,exit_status=1)
        assert lock.read_text()=="preserve unknown lock content" and manifest.read_bytes()==original+b"\n "
        lock.write_bytes(b"")
        self.passed("rebind refuses changed tool contracts and preserves a concurrent manifest edit")

        disposable=self.save("disposable_generated")
        missing=self.generate(disposable,self.record/"deleted-connection",tools=["read_document"])
        self.cli("connections","delete",disposable)
        self.launch(missing,"read_document","--id","weekly-fixture",exit_status=1)
        self.passed("a deleted connection fails explicitly without selecting another account")

        failed=self.launch(manifest,"read_document","--id","missing",exit_status=1)
        assert failed["error"]["code"]=="upstream_failed" and failed["result"]["output"]["isError"]
        self.definition(connection,url=f"http://127.0.0.1:{self.port}/mcp-rpc-error")
        failed=self.launch(manifest,"search_documents","--query","fixture",exit_status=1)
        assert failed["error"]["code"]=="upstream_failed" and "rpcError" in failed["result"]["output"]
        self.definition(connection,url=f"http://127.0.0.1:{self.port}/mcp-drop")
        before=self.stats()["calls"]
        failed=self.launch(manifest,"search_documents","--query","fixture",exit_status=1)
        assert "结果" in failed["error"]["message"] and self.stats()["calls"]==before+1
        self.passed("generated commands preserve tool/RPC failure semantics and send uncertain calls exactly once")

        runtime_path=self.data/"runtime.json";runtime_bytes=runtime_path.read_bytes();runtime=json.loads(runtime_bytes);posts=[]
        class Legacy(BaseHTTPRequestHandler):
            def log_message(self,*_): pass
            def do_GET(self):
                data=json.dumps({"application":"mcplink","instance":runtime["instance"],"managementApi":2}).encode()
                self.send_response(200);self.send_header("Content-Length",str(len(data)));self.end_headers();self.wfile.write(data)
            def do_POST(self): posts.append(True);self.send_response(500);self.end_headers()
        class LegacyServer(ThreadingHTTPServer):
            def server_bind(self):
                TCPServer.server_bind(self);self.server_name,self.server_port=self.server_address
        legacy=LegacyServer(("127.0.0.1",0),Legacy);thread=threading.Thread(target=legacy.serve_forever,daemon=True);thread.start()
        try:
            runtime_path.write_text(json.dumps(runtime|{"port":legacy.server_port}))
            failed=self.launch(manifest,"search_documents","--query","fixture",exit_status=1)
            assert "旧后台" in failed["error"]["message"] and not posts
            self.launch(manifest,"--help",help_text=True)
        finally:
            runtime_path.write_bytes(runtime_bytes);legacy.shutdown();legacy.server_close();thread.join()
        self.passed("a backend without dataset support is rejected before any execution RPC; generated help still works")

        oauth=self.cli("connections","save","--stdin",value={"name":"generated_oauth","definition":{"url":self.url,"oauth":True}})["id"]
        authorization=self.cli("credentials","login",oauth)
        assert urllib.request.urlopen(authorization["url"],timeout=15).status==200
        oauth_manifest=self.generate(oauth,self.record/"oauth-generated",tools=["read_document"])
        assert self.launch(oauth_manifest,"read_document","--id","weekly-fixture")["output"]["isError"] is False
        original=oauth_manifest.read_bytes();refreshes=self.stats()["refreshes"]
        time.sleep(4)
        assert self.launch(oauth_manifest,"read_document","--id","weekly-fixture")["output"]["isError"] is False
        assert self.stats()["refreshes"]>refreshes
        for client in self.clients: client.close()
        self.clients=[]
        runtime=json.loads((self.data/"runtime.json").read_text());os.kill(runtime["pid"],signal.SIGTERM)
        time.sleep(0.5)
        assert self.launch(oauth_manifest,"read_document","--id","weekly-fixture")["output"]["isError"] is False
        assert oauth_manifest.read_bytes()==original
        self.passed("generated commands reuse managed OAuth refresh and persist across backend restarts without regeneration")
        return {"connection":connection,"manifest_sha256":hashlib.sha256(manifest.read_bytes()).hexdigest()}

    def close(self, keep):
        for data in self.extra_data:
            try: os.kill(json.loads((data/"runtime.json").read_text())["pid"],signal.SIGTERM)
            except (FileNotFoundError,ProcessLookupError): pass
        super().close(keep)


def main():
    parser=argparse.ArgumentParser();parser.add_argument("--record",type=Path,required=True)
    record=parser.parse_args().record.resolve();record.mkdir(parents=True,exist_ok=False)
    harness=GeneratedCli(record);conclusion,error,ids="PASS",None,{}
    try: ids=harness.run()
    except Exception as exception:
        conclusion,error="FAIL",str(exception);traceback.print_exc()
    finally: harness.close(False)
    report={"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands,"ids":ids,"real_native_mcp_config_modified":False}
    (record/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(conclusion,record,flush=True)
    raise SystemExit(conclusion!="PASS")


if __name__=="__main__": main()
