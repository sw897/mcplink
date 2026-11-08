"""Exercise versioned OpenCode JSONC sources through the CLI and MCP Bridge."""
import argparse
from datetime import datetime,timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import signal
import socket
import subprocess
import sys
import time
import traceback

from tool_calls import ToolCalls
from run import BINARY,FIXTURE


class OpenCodeSources(ToolCalls):
    def run(self):
        schema=self.cli("schema")
        sources=next(c for c in schema["command"]["commands"] if c["name"]=="sources")
        add=next(c for c in sources["commands"] if c["name"]=="add")
        assert "opencode" in next(a for a in add["arguments"] if a["name"]=="agent")["values"]
        assert next(a for a in add["arguments"] if a["name"]=="layout")["values"]==["v1","v2"]
        skill=self.cli("skill","install","--agent","opencode")
        assert Path(skill["path"])==self.home/".config/opencode/skills/mcplink"
        assert self.cli("skill","install","--agent","opencode")["status"]=="unchanged"
        assert not self.data.exists()
        self.passed("OpenCode version selection and management Skill installation are discoverable offline")

        config=self.home/".config/opencode/opencode.jsonc"
        self.cli("sources","add","--agent","opencode",exit_status=1)
        assert not config.exists()
        source=self.cli("sources","add","--agent","opencode","--layout","v1")
        assert source["layout"]=="v1" and not config.exists()
        assert next(s for s in self.cli("status")["sources"] if s["id"]==source["id"])["layout"]=="v1"
        self.passed("an empty configuration requires an explicit layout and source registration does not create native files")

        process={"type":"local","command":[sys.executable,str(FIXTURE),"--stdio"],"environment":{"FIXTURE_PATH":"{env:MCPLINK_DATA_DIR}"},"enabled":True,"codemode":False}
        text='// synthetic leading comment\n{\n  "$schema": "https://opencode.ai/config.json",\n  "unknown_setting": {"keep":42}, // preserve this setting\n  "mcp": {\n    "native_fixture": '+json.dumps(process)+', // preserve other servers\n  },\n}\n'
        config.write_text(text)
        imported=self.cli("connections","import","--source",source["id"],"--name","native_fixture")["id"]
        definition=self.cli("connections","show",imported)["definition"]
        assert definition["command"]==sys.executable and definition["args"]==[str(FIXTURE),"--stdio"]
        assert definition["env"]["FIXTURE_PATH"]=="${env:MCPLINK_DATA_DIR}"
        assert self.cli("connections","probe",imported)["connection"]["status"]=="ready"
        self.passed("V1 JSONC imports native command arrays and environment references through the real MCP SDK")

        connection=self.save("shared_notes")
        plan=self.cli("changes","preview","--connection",connection,"--target",source["id"])
        assert config.read_text()==text
        result=self.cli("changes","apply",plan["planId"])
        actual=config.read_text();assert '// synthetic leading comment' in actual and '// preserve this setting' in actual and '// preserve other servers' in actual
        assert '"unknown_setting": {"keep":42}' in actual and '"enabled": true' in actual
        raw=self.cli("connections","import","--source",source["id"],"--name","shared_notes")
        assert raw["id"]==connection
        self.passed("V1 preview/apply preserves JSONC comments, unknown settings and unrelated servers")

        for action,expected in [("disable",False),("enable",True)]:
            plan=self.cli("changes","preview","--connection",connection,"--target",source["id"],"--action",action)
            self.cli("changes","apply",plan["planId"])
            status=self.cli("status")
            assert next(b for b in status["bindings"] if b["sourceId"]==source["id"] and b["name"]=="shared_notes")["enabled"]==expected
            assert '// preserve other servers' in config.read_text()
        self.passed("V1 enable and disable use its native enabled field without rewriting unrelated JSONC")

        project=self.record/"project-v2";project.mkdir()
        target=project/"opencode.jsonc"
        target.write_text('{\n// preserve V2 comment\n"mcp":{"timeout":{"catalog":30000},"servers":{}},\n"unrelated":{"variable":"{env:UNCHANGED_FIXTURE}"},\n}\n')
        modern=self.cli("sources","add","--agent","opencode","--scope","project","--project",str(project),"--layout","v2")
        self.cli("sources","add","--agent","opencode","--path",str(target),"--layout","v1",exit_status=1)
        assert modern["layout"]=="v2"
        assert next(s for s in self.cli("status")["sources"] if s["id"]==modern["id"])["layout"]=="v2"
        self.passed("V2 explicit project sources detect their native layout and reject incompatible version selection")

        self.cli("connections","probe",connection)
        plan=self.cli("changes","preview","--connection",connection,"--target",source["id"],"--target",modern["id"],"--mode","shared")
        results=self.cli("changes","apply",plan["planId"])["results"]
        assert all(v["status"]=="applied" for v in results)
        assert '"disabled": false' in target.read_text() and '"type": "local"' in target.read_text()
        assert '"environment"' in target.read_text() and '"env"' not in target.read_text()
        assert '"command": [' in target.read_text() and '"args"' not in target.read_text()
        assert "synthetic-fixture-authorization" not in target.read_text()+config.read_text()
        assert '"unrelated":{"variable":"{env:UNCHANGED_FIXTURE}"}' in target.read_text()
        self.passed("both layouts encode the shared Bridge with native command arrays and environment references, preserving V2 settings")

        for action,expected in [("disable",False),("enable",True)]:
            plan=self.cli("changes","preview","--connection",connection,"--target",modern["id"],"--action",action)
            self.cli("changes","apply",plan["planId"])
            status=self.cli("status")
            assert next(b for b in status["bindings"] if b["sourceId"]==modern["id"] and b["name"]=="shared_notes")["enabled"]==expected
            assert '"enabled"' not in target.read_text()
            assert '// preserve V2 comment' in target.read_text()
        self.passed("V2 enable and disable use disabled with inverse boolean semantics")

        clients=[self.bridge(connection,"opencode-shared-"+str(i)) for i in range(2)]
        def call(client):return client.call("tools/call",{"name":"read_document","arguments":{"id":"weekly-fixture"}})
        for client in clients:assert "result" in call(client)
        before={p:p.read_bytes() for p in [config,target]}
        self.cli("credentials","update",connection,"--stdin",value={"value":"synthetic-fixture-authorization"})
        for client in clients:assert "result" in call(client)
        runtime=json.loads((self.data/"runtime.json").read_text());os.kill(runtime["pid"],signal.SIGTERM);time.sleep(.4)
        assert not self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        for client in clients:assert "result" in call(client)
        assert all(path.read_bytes()==value for path,value in before.items())
        self.passed("credential updates and backend restart preserve both existing Bridges and native configuration references")

        plan=self.cli("changes","preview","--connection",connection,"--target",modern["id"],"--action","remove")
        removal=self.cli("changes","apply",plan["planId"])["results"][0]
        assert "shared_notes" not in target.read_text()
        self.cli("changes","restore",removal["operationId"])
        assert "shared_notes" in target.read_text() and '// preserve V2 comment' in target.read_text()
        self.passed("entry removal and restore preserve JSONC comments and surrounding settings")

        planned=self.cli("changes","preview","--connection",connection,"--target",modern["id"])
        target.write_text(target.read_text()+"// concurrent edit\n")
        self.cli("changes","apply",planned["planId"],exit_status=1)
        assert target.read_text().endswith('// concurrent edit\n')
        self.passed("JSONC changes retain concurrent-file conflict protection")

        invalid=self.record/"invalid";invalid.mkdir();invalid_file=invalid/"opencode.jsonc"
        for text in ['{"mcp":{"servers":{},"legacy":{"type":"local","command":["fixture"]}}}','{"mcp":{},"mcp":{}}',"{not_json: true}"]:
            invalid_file.write_text(text)
            self.cli("sources","add","--agent","opencode","--path",str(invalid_file),"--layout","v2",exit_status=1)
            assert invalid_file.read_text()==text
        self.passed("mixed layouts, duplicate properties and unsupported JSON extensions fail without rewriting files")

        unsupported={"unknown_policy":{"type":"remote","url":"https://example.invalid/mcp","future_native_policy":True},"native_codemode":{"type":"remote","url":"https://example.invalid/mcp","codemode":True},"literal_variable":{"type":"remote","url":"https://example.invalid/${MCPLINK_DATA_DIR}"},"file_macro":{"type":"remote","url":"https://example.invalid/mcp","headers":{"Authorization":"{file:synthetic-secret}"}},"modern_protocol":{"type":"remote","url":"https://example.invalid/mcp","protocol":"2026-07-28"},"native_oauth":{"type":"remote","url":"https://example.invalid/mcp","oauth":{"client_secret":"synthetic-special-secret"}},"native_timeout":{"type":"remote","url":"https://example.invalid/mcp","timeout":{"catalog":3456}}}
        invalid_file.write_text(json.dumps({"mcp":{"servers":unsupported}}))
        other=self.cli("sources","add","--agent","opencode","--path",str(invalid_file),"--layout","v2")
        for name in unsupported:
            error=self.cli("connections","import","--source",other["id"],"--name",name,exit_status=1)
            assert "synthetic-special-secret" not in json.dumps(error)
        invalid_file.write_text(json.dumps({"mcp":{"timeout":{"catalog":3456},"servers":{"uses_defaults":{"type":"local","command":[sys.executable,str(FIXTURE),"--stdio"]}}}}))
        self.cli("connections","import","--source",other["id"],"--name","uses_defaults",exit_status=1)
        self.passed("non-equivalent file macros, protocol, native OAuth and millisecond timeouts are explicit and do not expose values")

        natives=[]
        for version,variable,cwd,flags in [("V1","MCPLINK_E2E_OPENCODE_V1",self.record,[]),("V2","MCPLINK_E2E_OPENCODE_V2",project,["--standalone"])]:
            if executable:=os.environ.get(variable):
                environment=self.environment.copy()
                for name in ["OPENCODE_CONFIG","OPENCODE_CONFIG_CONTENT","OPENCODE_SERVER","OPENCODE_SERVER_USERNAME","OPENCODE_SERVER_PASSWORD"]:environment.pop(name,None)
                if version=="V2":
                    # 2.2.1's private one-shot CLI can list before its async MCP
                    # state is reconciled. Keep the native server alive and
                    # observe eventual readiness, as an interactive client does.
                    environment["OPENCODE_PASSWORD"]="synthetic-opencode-e2e"
                    with socket.socket() as listener:listener.bind(("127.0.0.1",0));port=listener.getsockname()[1]
                    with (self.record/"v2-server.log").open("w") as log:
                        server=subprocess.Popen([executable,"serve","--hostname","127.0.0.1","--port",str(port)],cwd=cwd,env=environment,stdout=log,stderr=subprocess.STDOUT)
                        try:
                            deadline=time.monotonic()+45
                            while True:
                                result=subprocess.run([executable,"--server",f"http://127.0.0.1:{port}","mcp","list"],cwd=cwd,env=environment,capture_output=True,text=True,timeout=30)
                                if any("shared_notes" in line and "connected" in line for line in result.stdout.splitlines()):break
                                assert time.monotonic()<deadline,result.stdout+result.stderr
                                time.sleep(.25)
                        finally:
                            server.terminate()
                            try:server.wait(timeout=10)
                            except subprocess.TimeoutExpired:server.kill();server.wait()
                else:
                    result=subprocess.run([executable,"mcp","list"],cwd=cwd,env=environment,capture_output=True,text=True,timeout=90)
                (self.record/(version.lower()+"-native-load.txt")).write_text(result.stdout+result.stderr)
                assert result.returncode==0 and "shared_notes" in result.stdout and "connected" in result.stdout.lower(),result.stdout+result.stderr
                native_version=subprocess.run([executable,"--version"],env=environment,capture_output=True,text=True,timeout=15).stdout.strip()
                natives.append({"layout":version,"version":native_version,"binary_sha256":hashlib.sha256(Path(executable).read_bytes()).hexdigest()})
        if natives:self.passed("specified native OpenCode clients load their respective configurations and connect through the shared Bridge",{"versions":natives,"scope":"native load and catalog, no model/tool execution claim"})
        return {"connection":connection,"v1_source":source["id"],"v2_source":modern["id"]}


def main():
    parser=argparse.ArgumentParser();parser.add_argument("--record",type=Path,required=True)
    record=parser.parse_args().record.resolve();record.mkdir(parents=True,exist_ok=False)
    harness=OpenCodeSources(record);conclusion,error,ids="PASS",None,{}
    try:ids=harness.run()
    except Exception as exception:conclusion,error="FAIL",str(exception);traceback.print_exc()
    finally:harness.close(False)
    report={"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands,"ids":ids,"real_native_mcp_config_modified":False}
    (record/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n");print(conclusion,record,flush=True);raise SystemExit(conclusion!="PASS")


if __name__=="__main__":main()
