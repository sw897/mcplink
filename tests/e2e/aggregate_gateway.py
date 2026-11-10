"""Exercise aggregate/single-service MCP entries and CLI with one managed OAuth grant."""
import argparse
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import time
import tomllib
import traceback
import urllib.request

from tool_calls import ToolCalls
from run import BINARY, ROOT


class AggregateGateway(ToolCalls):
    def __init__(self, record, pi_package=None):
        super().__init__(record)
        self.pi_package = pi_package

    def run(self):
        second_record = self.record / "second-upstream"
        second_record.mkdir()
        second = ToolCalls(second_record)
        try:
            first_id = self.save("documents", url=self.url)
            second_id = self.save("documents_other", url=second.url)
            login = self.cli("credentials", "login", first_id)
            assert urllib.request.urlopen(login["url"], timeout=15).status == 200
            self.cli("connections", "probe", first_id)
            self.cli("connections", "probe", second_id)
            registrations = self.stats()["registrations"]
            assert registrations == 1

            entry = self.cli("gateway", "save", "--stdin", value={
                "title": "Two upstreams", "connectionIds": [first_id, second_id],
            })
            entry_id = entry["id"]
            namespaces = {m["connectionId"]: m["namespace"] for m in entry["members"]}
            name = lambda connection, tool: namespaces[connection] + "__" + tool
            assert entry["name"] == "mcplink"
            detail = self.cli("connections", "show", entry_id)
            assert detail["definition"] is None and detail["connection"]["members"] == entry["members"]
            assert self.cli("gateway", "list")[0]["id"] == entry_id
            self.passed("one OAuth login creates a credential-free aggregate entry referencing two independent upstreams")

            paths = [self.home / ".codex/config.toml", self.home / ".hermes/config.yaml"]
            single_plan = self.cli("changes", "preview", "--connection", first_id, "--target", "hermes:user", "--mode", "shared")
            self.cli("changes", "apply", single_plan["planId"])
            before = [path.read_bytes() for path in paths]
            plan = self.cli("changes", "preview", "--connection", entry_id,
                            "--target", "codex:user", "--target", "hermes:user", "--mode", "shared", "--replace-shared")
            assert [path.read_bytes() for path in paths] == before
            applied = self.cli("changes", "apply", plan["planId"])
            assert all(row["status"] == "applied" for row in applied["results"])
            native = tomllib.loads(paths[0].read_text())["mcp_servers"]["mcplink"]
            assert native["args"] == ["bridge", "--connection", entry_id]
            assert first_id not in json.dumps(native) and second_id not in json.dumps(native)
            assert "documents:" not in paths[1].read_text() and "office_docs:" in paths[1].read_text()
            self.cli("changes", "preview", "--connection", entry_id, "--target", "codex:user", "--mode", "direct", exit_status=1)
            self.passed("preview/apply migrates selected single-service entries to one mcplink entry, preserves direct settings and rejects direct aggregate export")

            clients = [self.bridge(entry_id, "aggregate-agent-" + str(i)) for i in range(2)]
            single = self.bridge(first_id, "single-service")
            tools = clients[0].call("tools/list")["result"]["tools"]
            assert {tool["name"] for tool in tools} == {
                name(connection, tool) for connection in [first_id, second_id]
                for tool in ["read_document", "search_documents"]
            }
            assert len({tool["name"] for tool in tools}) == 4
            for connection, upstream in [(first_id, self), (second_id, second)]:
                previous = upstream.stats()["calls"]
                result = clients[0].call("tools/call", {"name": name(connection, "read_document"), "arguments": {"id": "weekly-fixture"}})
                assert "result" in result and upstream.stats()["calls"] == previous + 1
            self.passed("one live MCP entry discovers both upstreams and routes colliding tool names to the correct independent server")
            if self.pi_package:
                source = self.cli("sources", "add", "--agent", "pi")
                plan = self.cli("changes", "preview", "--connection", entry_id, "--target", source["id"], "--mode", "shared")
                self.cli("changes", "apply", plan["planId"])
                agent_dir = Path(source["path"]).parent
                project = self.record / "native-project"
                project.mkdir()
                native_tools = [name(first_id, "read_document"), name(second_id, "read_document")]
                result = subprocess.run(["node", str(ROOT / "tests/fixtures/pi-native-mcp.mjs"), str(self.pi_package.resolve()), str(agent_dir), str(project), "true", json.dumps(native_tools)], env=self.environment | {"PI_CODING_AGENT_DIR": str(agent_dir)}, capture_output=True, text=True, timeout=90)
                assert result.returncode == 0, (result.stdout, result.stderr)
                report = json.loads(result.stdout)
                assert len(report["servers"]) == 1 and report["servers"][0]["name"] == "mcplink" and report["servers"][0]["discoveredTools"] == 4
                (self.record / "native-pi.json").write_text(json.dumps(report, indent=2) + "\n")
                self.passed("the official Pi MCP runtime uses one native mcplink entry to discover and call both independent upstreams")

            for connection in [first_id, second_id]:
                prefix = "mcplink://" + namespaces[connection] + "/"
                resources = clients[0].call("resources/list")["result"]["resources"]
                resource = next(r for r in resources if r["uri"].startswith(prefix))
                read = clients[0].call("resources/read", {"uri": resource["uri"]})["result"]
                assert read["contents"][0]["uri"] == resource["uri"]
                templates = clients[0].call("resources/templates/list")["result"]["resourceTemplates"]
                assert any(t["uriTemplate"].startswith(prefix) and "{id}" in t["uriTemplate"] for t in templates)
                prompts = clients[0].call("prompts/list")["result"]["prompts"]
                prompt = next(p for p in prompts if p["name"].startswith(namespaces[connection] + "__"))
                assert "result" in clients[0].call("prompts/get", {"name": prompt["name"], "arguments": {"id": "weekly-fixture"}})
            self.passed("resource URIs, templates and prompt names are namespaced and reversibly routed")

            generated = self.cli("cli", "generate", first_id, "--name", "documents-cli",
                                 "--output", str(self.record / "generated"), "--tool", "read_document")
            time.sleep(4)
            def aggregate_call(client):
                return client.call("tools/call", {"name": name(first_id, "read_document"), "arguments": {"id": "weekly-fixture"}})
            with ThreadPoolExecutor(max_workers=4) as workers:
                pending = [workers.submit(aggregate_call, client) for client in clients]
                pending.append(workers.submit(single.call, "tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}}))
                pending.append(workers.submit(self.cli, "cli", "run", "--manifest", generated["manifest"], "--", "read_document", "--stdin", value={"id": "weekly-fixture"}))
                values = [future.result(timeout=45) for future in pending]
            assert all("result" in value for value in values[:3]) and not values[3]["output"]["isError"]
            assert self.stats()["registrations"] == registrations and self.stats()["refreshes"] > 0
            self.passed("two aggregate clients, a single-service client and generated CLI share the original rotating OAuth grant without another login")
            aggregate_cli = self.cli("cli", "generate", entry_id, "--name", "aggregate-cli", "--output", str(self.record / "generated-aggregate"), "--tool", name(first_id, "read_document"))
            self.cli("gateway", "stop")
            assert "result" in aggregate_call(clients[0])
            assert "result" in single.call("tools/call", {"name":"read_document","arguments":{"id":"weekly-fixture"}})
            assert not self.cli("cli", "run", "--manifest", aggregate_cli["manifest"], "--", name(first_id, "read_document"), "--stdin", value={"id":"weekly-fixture"})["output"]["isError"]
            assert self.stats()["registrations"] == registrations
            self.passed("daemon restart restores the same grant for established MCP clients and generated aggregate CLI")

            self.cli("credentials", "disconnect", first_id)
            assert "error" in aggregate_call(clients[0])
            assert "error" in single.call("tools/call", {"name":"read_document","arguments":{"id":"weekly-fixture"}})
            self.cli("cli", "run", "--manifest", generated["manifest"], "--", "read_document", "--stdin", value={"id":"weekly-fixture"}, exit_status=1)
            self.cli("credentials", "update", first_id, "--stdin", value={"value":"synthetic-fixture-authorization"})
            assert "result" in aggregate_call(clients[0]) and "result" in single.call("tools/call", {"name":"read_document","arguments":{"id":"weekly-fixture"}})
            assert not self.cli("cli", "run", "--manifest", generated["manifest"], "--", "read_document", "--stdin", value={"id":"weekly-fixture"})["output"]["isError"]
            self.passed("one managed disconnect stops all three paths; one credential update restores them without rewriting clients or CLI")

            other_entry = self.cli("gateway", "save", "--stdin", value={"title": "Other Agent", "connectionIds": [second_id]})
            other_client = self.bridge(other_entry["id"], "restricted-agent")
            assert len(other_client.call("tools/list")["result"]["tools"]) == 2
            self.cli("connections", "delete", second_id, exit_status=1)
            stale = entry
            stale_plan = self.cli("changes", "preview", "--connection", entry_id, "--target", "hermes:user", "--action", "remove")
            entry = self.cli("gateway", "save", "--stdin", value={"id": entry_id, "editVersion": entry["editVersion"], "title": entry["title"], "connectionIds": [first_id]})
            failed_plan = self.cli("changes", "apply", stale_plan["planId"], exit_status=1)
            assert all(row["status"] == "failed" for row in failed_plan["result"]["results"])
            self.cli("gateway", "save", "--stdin", value={"id": entry_id, "editVersion": stale["editVersion"], "connectionIds": [second_id]}, exit_status=1)
            assert len(clients[0].call("tools/list")["result"]["tools"]) == 2
            assert "error" in clients[0].call("tools/call", {"name": name(second_id, "read_document"), "arguments": {"id": "weekly-fixture"}})
            assert len(other_client.call("tools/list")["result"]["tools"]) == 2
            self.passed("per-Agent views remain independent; live selection changes revoke removed routes and stale edits fail")

            entry = self.cli("gateway", "save", "--stdin", value={"id": entry_id, "editVersion": entry["editVersion"], "connectionIds": [first_id, second_id]})
            self.cli("credentials", "disconnect", second_id)
            listed = clients[1].call("tools/list")["result"]
            assert len(listed["tools"]) == 2 and listed["_meta"]["mcplink"]["upstreamErrors"]
            inspected = self.cli("connections", "tools", entry_id)
            assert len(inspected["upstreamErrors"]) == 1 and inspected["upstreamErrors"][0]["connectionId"] == second_id
            assert inspected["connection"]["lastError"] and inspected["connection"]["toolCount"] == 2
            assert "result" in aggregate_call(clients[1])
            self.passed("an unavailable upstream is reported without blocking healthy discovery and calls")

            self.cli("gateway", "save", "--stdin", value={"id": entry_id, "editVersion": entry["editVersion"], "connectionIds": []})
            assert clients[0].call("tools/list")["result"]["tools"] == []
            assert "error" in aggregate_call(clients[0])
            assert "result" in single.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
            self.passed("an empty aggregate view exposes no services and leaves the single-service grant usable")

            many_id = self.save("many_tools", url=f"http://127.0.0.1:{self.port}/mcp-many")
            many_entry = self.cli("gateway", "save", "--stdin", value={"connectionIds":[many_id]})
            many_client = self.bridge(many_entry["id"], "paged-aggregate")
            page = many_client.call("tools/list")["result"]
            assert len(page["tools"]) == 100 and page["nextCursor"]
            rest = many_client.call("tools/list", {"cursor":page["nextCursor"]})["result"]
            assert len(rest["tools"]) == 32 and "nextCursor" not in rest
            all_tools = page["tools"] + rest["tools"]
            assert len({t["name"] for t in all_tools}) == 132 and all(len(t["name"]) <= 128 for t in all_tools)
            for tool in rest["tools"][-2:]:
                assert "result" in many_client.call("tools/call", {"name":tool["name"],"arguments":{"id":"weekly-fixture"}})
            assert "error" in many_client.call("tools/list", {"cursor":"invalid"})
            self.cli("gateway", "save", "--stdin", value={"id":many_entry["id"],"editVersion":many_entry["editVersion"],"connectionIds":[]})
            assert "error" in many_client.call("tools/list", {"cursor":page["nextCursor"]})
            self.passed("upstream and aggregate pagination preserve all tools; long and special names route correctly and invalid/stale cursors fail")

            links_id = self.save("resource_links", url=f"http://127.0.0.1:{self.port}/mcp-links")
            links = self.cli("gateway", "save", "--stdin", value={"connectionIds":[links_id]})
            link_client = self.bridge(links["id"], "resource-link-aggregate")
            tool = link_client.call("tools/list")["result"]["tools"][0]["name"]
            content = link_client.call("tools/call", {"name":tool,"arguments":{"query":"fixture"}})["result"]["content"]
            prefix = "mcplink://" + links["members"][0]["namespace"] + "/"
            assert content[0]["uri"].startswith(prefix) and content[1]["resource"]["uri"].startswith(prefix)
            self.cli("connections", "delete", links["id"])
            assert self.cli("connections", "show", links_id)["connection"]["active"]
            self.passed("resource links and embedded resource contents carry routed URIs; deleting an unbound entry retains upstream authentication")

            for selected in [[many_entry["id"]], [first_id, first_id], ["missing"]]:
                self.cli("gateway", "save", "--stdin", value={"connectionIds":selected}, exit_status=1)
            self.cli("gateway", "save", "--stdin", value={"id":42,"connectionIds":[]}, exit_status=1)
            self.passed("nested entries, duplicate members and missing connections cannot create invalid aggregate views")

            for index, path in enumerate(paths):
                assert path.read_bytes().count(b"synthetic-fixture-authorization") == before[index].count(b"synthetic-fixture-authorization")
            for path in [Path(generated["manifest"]), self.data / "mcplink.sqlite3"]:
                data = path.read_bytes()
                assert b"synthetic-fixture-authorization" not in data and b"access_token" not in data
            self.passed("aggregate configuration, generated artifacts and SQLite contain no upstream credentials; existing native settings are preserved")
            return {"entry": entry_id, "upstreams": [first_id, second_id]}
        finally:
            second.close(False)


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--pi-package", type=Path)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = AggregateGateway(record, args.pi_package)
    conclusion, error, ids = "PASS", None, {}
    try:
        ids = harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(False)
    report = {"conclusion": conclusion, "error": error, "executed_at": datetime.now(timezone.utc).isoformat(), "os": platform.platform(), "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(), "scenarios": harness.results, "commands": harness.commands, "ids": ids, "real_native_mcp_config_modified": False}
    (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
