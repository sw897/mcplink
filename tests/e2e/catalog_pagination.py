"""Verify stable aggregate pagination against changing upstream list metadata/order."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import traceback

from tool_calls import ToolCalls
from run import BINARY


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

        self.cli("credentials", "update", connection, "--stdin", value={"value":"synthetic-fixture-authorization"})
        assert "error" in client.call("tools/list", {"cursor":fresh["nextCursor"]})
        again = client.call("tools/list")["result"]
        self.cli("gateway", "save", "--stdin", value={"id":entry["id"],"editVersion":entry["editVersion"],"connectionIds":[]})
        assert "error" in client.call("tools/list", {"cursor":again["nextCursor"]})
        self.passed("member credentials and entry selection changes revoke prior catalog snapshots")


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
