"""Reproduce remote legacy HTTP/SSE version/session behavior at real CLI entry points."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime,timezone
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import hashlib
from ipaddress import IPv6Address
import json
from pathlib import Path
import platform
import threading
import traceback
from urllib.parse import urlsplit

from tool_calls import ToolCalls
from run import BINARY


class RemoteTransports(ToolCalls):
    def outbound_policy(self):
        proxy_requests = []

        class Proxy(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def reject(self):
                proxy_requests.append(True)
                self.send_error(502)

            do_GET = do_POST = do_CONNECT = reject

        with ThreadingHTTPServer(("127.0.0.1", 0), Proxy) as proxy:
            thread = threading.Thread(target=proxy.serve_forever, daemon=True)
            thread.start()
            address = f"http://127.0.0.1:{proxy.server_port}"
            for key in ("HTTP_PROXY","HTTPS_PROXY","ALL_PROXY","http_proxy","https_proxy","all_proxy"):
                self.environment[key] = address
            for key in ("NO_PROXY","no_proxy"):
                self.environment.pop(key, None)
            try:
                for name, host in [("local_dns", "localhost"), ("mapped_loopback", "[::ffff:127.0.0.1]")]:
                    endpoint = f"http://{host}:{self.port}/mcp"
                    connection = self.save(name, url=endpoint)
                    stored = self.cli("connections","show",connection)["definition"]["url"]
                    if name == "mapped_loopback":
                        assert str(IPv6Address(urlsplit(stored).hostname).ipv4_mapped) == "127.0.0.1"
                    assert self.cli("connections","probe",connection)["connection"]["status"] == "ready"
                    assert self.cli("connections","show",connection)["definition"]["url"] == stored
                management = json.loads((self.data/"runtime.json").read_text())["port"]
                for name, host in [("management_target", "127.0.0.1"), ("management_mapped", "[::ffff:127.0.0.1]")]:
                    connection = self.save(name, url=f"http://{host}:{management}/mcp")
                    result = self.cli("connections","probe",connection,exit_status=1)
                    assert "本机管理端口" in result["result"]["error"]
                assert not proxy_requests
            finally:
                proxy.shutdown()
                thread.join()
        self.passed("explicit loopback hostnames and mapped loopback connect directly; the protected daemon port and synthetic proxy receive no upstream calls")

        domain = "https://api.fixture.invalid:8443"
        private = self.save("private_confirmation", url=domain+"/mcp", privateNetworkOrigin=domain)
        current = self.cli("connections","show",private)
        assert current["definition"]["privateNetworkOrigin"] == domain
        invalid = self.cli("connections","save","--stdin",value={"name":"different_target",
            "definition":{"url":domain+"/mcp","privateNetworkOrigin":"https://other.fixture.invalid:8443"}},exit_status=1)
        assert "完整 HTTPS 来源" in invalid["error"]["message"]
        # Offline editing proves the confirmation belongs to the chosen origin.
        self.cli("connections","save","--stdin",value={"id":private,"name":"private_confirmation",
            "definition":current["definition"] | {"url":"https://other.fixture.invalid:8443/mcp"}})
        assert "privateNetworkOrigin" not in self.cli("connections","show",private)["definition"]
        self.passed("private HTTPS approval is persisted only for the exact origin and drops when the origin changes")

    def replace_during_initialize(self):
        connection = self.save("slow_initialize", url=self.url, startupTimeout=30)
        previous = self.stats()["gates"].get("initialize", 0)
        self.control("initialize", "arm")
        with ThreadPoolExecutor(max_workers=3) as workers:
            started = workers.submit(self.cli, "mcp", "describe", connection, exit_status=1)
            try:
                self.wait_gate("initialize", previous)
                workers.submit(self.cli,"credentials","update",connection,"--stdin",
                               value={"value":"synthetic-fixture-authorization"}).result(timeout=5)
                workers.submit(self.save,"independent_init",url=self.url).result(timeout=5)
            finally:
                self.control("initialize", "release")
            assert "更新" in started.result(timeout=10)["error"]["message"]
        assert "result" in self.bridge(connection, "new-generation").call("tools/list")
        assert not self.invoke(connection,"read_document",{"id":"weekly-fixture"})["output"]["isError"]
        self.passed("a credential change retires a starting session while independent management and the new generation stay available")

    def run(self):
        self.outbound_policy()
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
        self.replace_during_initialize()
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
