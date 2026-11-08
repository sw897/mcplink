"""Exercise scoped Claude JSON writes through the real CLI and transaction core."""
import argparse
from datetime import datetime,timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import subprocess
import sys
import traceback

from tool_calls import ToolCalls
from run import BINARY,FIXTURE


class ClaudeWritable(ToolCalls):
    def run(self):
        home=self.home/".claude";home.mkdir(exist_ok=True)
        user=home/".claude.json"
        original='{"preferences": {"synthetic": 42}, "mcpServers": {}, "projects": {"unrelated-project": {"allowedTools": ["fixture"], "mcpServers": {}}}}\n'
        user.write_text(original)
        source=next(s for s in self.cli("sources","list") if s["id"]=="claude:user")
        assert source["scope"]=="user" and not source["readOnly"]
        self.passed("Claude user configuration is writable and exposes its explicit scope")

        connection=self.save("scoped_notes")
        plan=self.cli("changes","preview","--connection",connection,"--target","claude:user")
        assert user.read_text()==original
        applied=self.cli("changes","apply",plan["planId"])
        assert applied["results"][0]["status"]=="applied"
        document=json.loads(user.read_text());entry=document["mcpServers"]["scoped_notes"]
        assert entry["type"]=="http" and entry["url"].endswith("/mcp-tools") and "enabled" not in entry
        assert '"preferences": {"synthetic": 42}' in user.read_text()
        assert document["projects"]["unrelated-project"]["allowedTools"]==["fixture"]
        self.passed("user preview is read-only and apply writes native HTTP fields while preserving preferences and formatting")

        project=self.record/"project with spaces";project.mkdir()
        subprocess.run(["git","init","--quiet",str(project)],check=True,capture_output=True)
        project_key=str(project).replace('\\','/') if os.name=="nt" else str(project)
        child=project/"child";child.mkdir()
        native_scope_contract=None
        if executable:=os.environ.get("MCPLINK_E2E_CLAUDE"):
            native_home=self.record/"native-scope-contract";native_home.mkdir()
            native_env=self.environment|{"CLAUDE_CONFIG_DIR":str(native_home),"PWD":str(project)}
            result=subprocess.run([executable,"mcp","add","--scope","local","--transport","stdio","native_scope_contract","--",sys.executable,str(FIXTURE),"--stdio"],cwd=project,env=native_env,capture_output=True,text=True,timeout=35)
            assert result.returncode==0,result.stdout+result.stderr
            native_scope_contract=json.loads((native_home/".claude.json").read_text())["projects"]
            assert project_key in native_scope_contract,list(native_scope_contract)
        local=self.cli("sources","add","--agent","claude","--scope","local","--project",str(child))
        shared=self.cli("sources","add","--agent","claude","--scope","project","--project",str(project))
        assert Path(local["project"])==project and Path(local["path"])==user
        assert Path(shared["path"])==project/".mcp.json" and not shared["readOnly"]
        again=self.cli("sources","add","--agent","claude","--scope","local","--project",str(project))
        assert again["id"]==local["id"]
        self.passed("explicit local/project sources use native locations, normalize Git roots and remain distinct despite a shared user file")

        before_projects=json.loads(user.read_text())["projects"]
        plan=self.cli("changes","preview","--connection",connection,"--target","claude:user","--target",local["id"],"--target",shared["id"])
        results=self.cli("changes","apply",plan["planId"])["results"]
        assert len(results)==3 and all(v["status"]=="applied" for v in results)
        document=json.loads(user.read_text())
        assert document["projects"][project_key]["mcpServers"]["scoped_notes"]==entry
        assert document["projects"]["unrelated-project"]==before_projects["unrelated-project"]
        assert "hasTrustDialogAccepted" not in document["projects"][project_key]
        assert json.loads((project/".mcp.json").read_text())["mcpServers"]["scoped_notes"]==entry
        rows=self.cli("status")["bindings"]
        user_row=next(row for row in rows if row["sourceId"]=="claude:user" and row["name"]=="scoped_notes")
        assert {row["sourceId"] for row in user_row["overriddenIn"]}=={local["id"],shared["id"]}
        self.passed("one batch writes user, local and project entries with the same name without altering approval or trust preferences")

        local_op=next(v["operationId"] for v in results if v["sourceId"]==local["id"])
        self.cli("changes","restore",local_op)
        document=json.loads(user.read_text())
        assert "scoped_notes" not in document["projects"][project_key]["mcpServers"]
        assert document["mcpServers"]["scoped_notes"]==entry
        assert json.loads((project/".mcp.json").read_text())["mcpServers"]["scoped_notes"]==entry
        self.passed("restoring one scoped entry preserves the same-name user/project entries and unrelated preferences")

        plan=self.cli("changes","preview","--connection",connection,"--target",local["id"])
        user.write_text(user.read_text()+" ")
        failed=self.cli("changes","apply",plan["planId"],exit_status=1)
        assert failed["result"]["results"][0]["status"]=="failed"
        assert "scoped_notes" not in json.loads(user.read_text())["projects"][project_key]["mcpServers"]
        self.passed("scope writes retain the existing concurrent-file conflict guard")

        for action in ["disable","enable"]:
            failure=self.cli("changes","preview","--connection",connection,"--target","claude:user","--action",action,exit_status=1)
            assert "项目" in failure["error"]["message"]
        filtered=self.save("filtered_for_claude",includeTools=["read_document"])
        self.cli("changes","preview","--connection",filtered,"--target","claude:user",exit_status=1)
        remote_variables=self.save("remote_variables_for_claude",headers={"X-Fixture":"${env:MCPLINK_DATA_DIR}"})
        self.cli("changes","preview","--connection",remote_variables,"--target","claude:user",exit_status=1)
        self.passed("unsupported native enable/disable and tool-filter semantics fail before writing misleading server fields")

        valid=user.read_bytes()
        for invalid in ['{"mcpServers":{},"mcpServers":{}}','[]','{"mcpServers": []}']:
            user.write_text(invalid)
            self.cli("changes","preview","--connection",connection,"--target","claude:user",exit_status=1)
            assert user.read_text()==invalid
        user.write_bytes(valid)
        user.write_text(json.dumps({"mcpServers":{"native_helper":{"type":"http","url":"https://example.invalid/mcp","headersHelper":"synthetic-helper-private-value"}}}))
        failure=self.cli("connections","import","--source","claude:user","--name","native_helper",exit_status=1)
        assert "synthetic-helper-private-value" not in json.dumps(failure)
        for native in [{"type":"http","url":"https://example.invalid/mcp","timeout":2000},{"type":"http","url":"https://example.invalid/mcp","headers":{"Authorization":"${NPM_TOKEN:-synthetic-secret-fallback}"}},{"type":"http","url":"https://example.invalid/mcp","future_native_policy":True}]:
            user.write_text(json.dumps({"mcpServers":{"unsupported_native":native}}))
            failure=self.cli("connections","import","--source","claude:user","--name","unsupported_native",exit_status=1)
            assert "synthetic-secret-fallback" not in json.dumps(failure)
        user.write_bytes(valid)
        self.cli("sources","add","--agent","claude","--scope","local","--project",str(project),"--path",str(project/"wrong.json"),exit_status=1)
        self.passed("malformed/duplicate JSON and mismatched scoped locations are rejected without overwriting configuration")

        variables={"type":"stdio","command":"${MCPLINK_CLAUDE_E2E_PYTHON:-"+sys.executable+"}","args":[str(FIXTURE),"--stdio"],"env":{"FIXTURE_PATH":"${MCPLINK_DATA_DIR}","RESERVED_NAME":"${userHome:-synthetic-secret-fallback}"}}
        document=json.loads(user.read_text());document["mcpServers"]["native_variables"]=variables;user.write_text(json.dumps(document))
        variable_id=self.cli("connections","import","--source","claude:user","--name","native_variables")["id"]
        normalized=self.cli("connections","show",variable_id)["definition"]
        assert normalized["command"]==variables["command"].replace("${","${env:")
        assert normalized["env"]["FIXTURE_PATH"]=="${env:MCPLINK_DATA_DIR}" and normalized["env"]["RESERVED_NAME"]=="••••••••"
        assert "synthetic-secret-fallback" not in json.dumps(normalized)
        self.cli("connections","save","--stdin",value={"id":variable_id,"name":"native_variables","definition":normalized})
        assert self.cli("connections","probe",variable_id)["connection"]["status"]=="ready"
        plan=self.cli("changes","preview","--connection",variable_id,"--target",local["id"])
        self.cli("changes","apply",plan["planId"])
        encoded=json.loads(user.read_text())["projects"][project_key]["mcpServers"]["native_variables"]
        assert encoded==variables
        self.passed("Claude native variables normalize and round-trip without confusing reserved environment names with managed project variables")

        managed=self.record/"managed-mcp.json";managed.write_text('{"mcpServers":{}}')
        protected=self.cli("sources","add","--agent","claude","--scope","user","--path",str(managed))
        assert protected["readOnly"] and protected["scope"]=="managed"
        duplicate=self.cli("sources","add","--agent","hermes","--path",str(managed))
        assert duplicate["id"]==protected["id"] and duplicate["readOnly"]
        self.cli("changes","preview","--connection",connection,"--target",protected["id"],exit_status=1)
        assert managed.read_text()=='{"mcpServers":{}}'
        self.passed("managed configuration stays read-only across explicit scope and duplicate cross-Agent registration")

        self.cli("connections","probe",connection)
        plan=self.cli("changes","preview","--connection",connection,"--target","claude:user","--target",local["id"],"--mode","shared")
        assert all(v["status"]=="applied" for v in self.cli("changes","apply",plan["planId"])["results"])
        document=json.loads(user.read_text());native=document["mcpServers"]["scoped_notes"]
        assert native["type"]=="stdio" and native["args"]==["bridge","--connection",connection] and "enabled" not in native
        assert "synthetic-fixture-authorization" not in json.dumps(native)
        clients=[self.bridge(connection,"claude-scoped-"+str(i)) for i in range(2)]
        for client in clients: assert "result" in client.call("tools/call",{"name":"read_document","arguments":{"id":"weekly-fixture"}})
        self.cli("credentials","update",connection,"--stdin",value={"value":"synthetic-fixture-authorization"})
        for client in clients: assert "result" in client.call("tools/call",{"name":"read_document","arguments":{"id":"weekly-fixture"}})
        self.passed("Claude shared entries use native stdio fields and credential references; existing Bridges survive managed updates")

        local_file=project/".mcp.json";unrelated=project/"relative.py";shutil.copy2(FIXTURE,unrelated)
        local_file.write_text(json.dumps({"mcpServers":{"relative_fixture":{"type":"stdio","command":sys.executable,"args":["relative.py","--stdio"]}}}))
        relative=self.cli("connections","import","--source",shared["id"],"--name","relative_fixture")["id"]
        assert self.cli("connections","probe",relative)["connection"]["status"]=="ready"
        self.cli("changes","preview","--connection",relative,"--target","claude:user",exit_status=1)
        self.passed("imported project commands execute in their source project and refuse context-changing native export")

        if executable:=os.environ.get("MCPLINK_E2E_CLAUDE"):
            version=subprocess.run([executable,"--version"],env=self.environment,capture_output=True,text=True,timeout=15)
            native_env=self.environment|{"PWD":str(project)}
            result=subprocess.run([executable,"mcp","get","scoped_notes"],cwd=project,env=native_env,capture_output=True,text=True,timeout=50)
            assert result.returncode==0 and "Connected" in result.stdout and "Local" in result.stdout,result.stdout+result.stderr
            direct=subprocess.run([executable,"mcp","get","native_variables"],cwd=project,env=native_env,capture_output=True,text=True,timeout=50)
            assert direct.returncode==0 and "Connected" in direct.stdout,direct.stdout+direct.stderr
            self.passed("the specified native Claude CLI loads the local shared entry and completes its MCP health check",{"version":version.stdout.strip(),"native_project_keys":list(native_scope_contract),"scope":"native configuration load and MCP health, no model/tool execution claim"})
        return {"connection":connection,"local_source":local["id"],"project_source":shared["id"]}


def main():
    parser=argparse.ArgumentParser();parser.add_argument("--record",type=Path,required=True)
    record=parser.parse_args().record.resolve();record.mkdir(parents=True,exist_ok=False)
    harness=ClaudeWritable(record);conclusion,error,ids="PASS",None,{}
    try: ids=harness.run()
    except Exception as exception: conclusion,error="FAIL",str(exception);traceback.print_exc()
    finally: harness.close(False)
    report={"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands,"ids":ids,"native_client_requested":bool(os.environ.get("MCPLINK_E2E_CLAUDE")),"real_native_mcp_config_modified":False}
    (record/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(conclusion,record,flush=True);raise SystemExit(conclusion!="PASS")


if __name__=="__main__": main()
