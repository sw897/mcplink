"""Verify stable aggregate pagination against changing upstream list metadata/order."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import sys
import traceback

from tool_calls import ToolCalls
from run import BINARY, ROOT


class CatalogPagination(ToolCalls):
    def run(self):
        connection = self.save("unstable_catalog", url=f"http://127.0.0.1:{self.port}/mcp-unstable")
        entry = self.cli("gateway", "save", "--stdin", value={"connectionIds":[connection]})
        client = self.bridge(entry["id"], "stable-pagination")
        first = client.call("tools/list")["result"]
        reads = self.stats()["lists"]
        second = client.call("tools/list", {"cursor":first["nextCursor"]})
        assert "result" in second, second
        second = second["result"]
        rows = first["tools"] + second["tools"]
        assert len(first["tools"]) == 100 and len(second["tools"]) == 32
        assert len({r["name"] for r in rows}) == 132
        assert len({r["description"] for r in rows}) == 1
        assert self.stats()["lists"] == reads
        self.passed("one aggregate page walk uses an immutable catalog without rediscovering changing upstream metadata")

        fresh = client.call("tools/list")["result"]
        assert fresh["tools"][0]["description"] != first["tools"][0]["description"]
        self.passed("a new first page discovers the current upstream catalog")
        other = self.cli("gateway", "save", "--stdin", value={"connectionIds":[connection]})
        foreign = self.bridge(other["id"], "foreign-cursor")
        assert "error" in foreign.call("tools/list", {"cursor":fresh["nextCursor"]})
        assert "error" in client.call("prompts/list", {"cursor":fresh["nextCursor"]})
        self.passed("catalog cursors cannot cross entry or capability boundaries")

        self.cli("gateway", "refresh", entry["id"])
        assert "error" in client.call("tools/list", {"cursor":fresh["nextCursor"]})
        fresh = client.call("tools/list")["result"]
        self.cli("gateway", "refresh", connection)
        assert "error" in client.call("tools/list", {"cursor":fresh["nextCursor"]})
        fresh = client.call("tools/list")["result"]
        self.passed("explicit refresh of the entry or its member invalidates existing cursors")

        self.cli("credentials", "update", connection, "--stdin", value={"value":"synthetic-fixture-authorization"})
        assert "error" in client.call("tools/list", {"cursor":fresh["nextCursor"]})
        again = client.call("tools/list")["result"]
        self.cli("gateway", "save", "--stdin", value={"id":entry["id"],"editVersion":entry["editVersion"],"connectionIds":[]})
        assert "error" in client.call("tools/list", {"cursor":again["nextCursor"]})
        self.passed("member credentials and entry selection changes revoke prior catalog snapshots")

        control = self.record / "paged-catalog.json"
        state = {"tools":0,"resources":0,"prompts":0}
        control.write_text(json.dumps(state))
        upstream = self.cli("connections","save","--stdin",value={"name":"paged_notifications","definition":{
            "command":sys.executable,"args":[str(ROOT/"tests/fixtures/catalog_server.py"),str(control),"132"]}})["id"]
        notified = self.cli("gateway","save","--stdin",value={"connectionIds":[upstream]})
        fresh_client = self.bridge(notified["id"], "paged-list-changed")
        tools = fresh_client.call("tools/list")["result"]["nextCursor"]
        resources = fresh_client.call("resources/list")["result"]["nextCursor"]
        updated = control.with_suffix(".tmp")
        updated.write_text(json.dumps(state | {"tools":1}))
        updated.replace(control)
        fresh_client.wait_notification("notifications/tools/list_changed")
        assert "error" in fresh_client.call("tools/list", {"cursor":tools})
        assert len(fresh_client.call("resources/list", {"cursor":resources})["result"]["resources"]) == 32
        self.passed("an upstream tool notification expires tool pages while the resource snapshot stays valid")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = CatalogPagination(record)
    conclusion, error = "PASS", None
    try:
        harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(False)
    (record / "report.json").write_text(json.dumps({"conclusion":conclusion,"error":error,"executed_at":datetime.now(timezone.utc).isoformat(),"os":platform.platform(),"binary_sha256":hashlib.sha256(BINARY.read_bytes()).hexdigest(),"scenarios":harness.results,"commands":harness.commands},indent=2)+"\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
