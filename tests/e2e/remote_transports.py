"""Reproduce remote legacy HTTP/SSE version/session behavior at real CLI entry points."""
import argparse
from datetime import datetime,timezone
import hashlib
import json
from pathlib import Path
import platform
import traceback

from tool_calls import ToolCalls
from run import BINARY


class RemoteTransports(ToolCalls):
    def run(self):
        connection=self.save("remote_legacy_session",url=f"http://127.0.0.1:{self.port}/mcp-session")
        result=self.cli("connections","probe",connection)
        assert result["connection"]["status"]=="ready"
        description=self.cli("mcp","describe",connection)
        assert description["protocolVersion"]=="2025-11-25"
        versions=self.stats()["sessionVersions"]
        assert versions and set(versions)=={"2025-11-25"}
        sessions=len(versions)
        self.passed("legacy initialization uses the implemented protocol and preserves the SSE session through a GET 405")

        direct=self.invoke(connection,"read_document",{"id":"weekly-fixture"})
        clients=[self.bridge(connection,"remote-legacy-"+str(index)) for index in range(2)]
        for bridge in clients:
            assert bridge.call("tools/call",{"name":"read_document","arguments":{"id":"weekly-fixture"}})["result"]==direct["output"]
        assert len(self.stats()["sessionVersions"])==sessions
        self.passed("two established Bridges and the typed CLI reuse one authenticated upstream session")

        self.cli("credentials","update",connection,"--stdin",value={"value":"synthetic-fixture-authorization"})
        for bridge in clients:
            assert bridge.call("tools/call",{"name":"read_document","arguments":{"id":"weekly-fixture"}})["result"]==direct["output"]
        assert self.stats()["sessionVersions"]==["2025-11-25"]*(sessions+1)
        self.passed("a managed credential update reconnects both existing Bridges with a fresh valid legacy session")

        self.cli("gateway","pause")
        self.invoke(connection,"read_document",{"id":"weekly-fixture"},exit_status=1)
        self.cli("gateway","resume")
        assert self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]==direct["output"]
        self.passed("pause and resume keep the session/version contract and invocation semantics")
        return {"connection":connection}


def main():
    parser=argparse.ArgumentParser();parser.add_argument("--record",type=Path,required=True)
    record=parser.parse_args().record.resolve();record.mkdir(parents=True,exist_ok=False)
    harness=RemoteTransports(record);conclusion,error,ids="PASS",None,{}
    try: ids=harness.run()
    except Exception as exception:
        conclusion,error="FAIL",str(exception);traceback.print_exc()
    finally: harness.close(False)
    report={"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"script_sha256":hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands,"ids":ids,"real_native_mcp_config_modified":False}
    (record/"report.json").write_text(json.dumps(report,ensure_ascii=False,indent=2)+"\n")
    print(conclusion,record,flush=True);raise SystemExit(conclusion!="PASS")


if __name__=="__main__": main()
