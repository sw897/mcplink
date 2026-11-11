"""Verify scoped list-change notifications without polling business tools."""
import argparse
import hashlib
import json
from pathlib import Path
import sys
import subprocess
import time
import traceback

from tool_calls import ToolCalls
from run import BINARY, ROOT


class CatalogRefresh(ToolCalls):
    def __init__(self, record, pi_package=None):
        super().__init__(record)
        self.pi_package = pi_package

    def run(self):
        control = self.record / "catalog-control.json"
        control.write_text(json.dumps({"tools": 0, "resources": 0, "prompts": 0}))
        upstream = self.cli("connections", "save", "--stdin", value={"name": "changing_catalog", "definition": {"command": sys.executable, "args": [str(ROOT / "tests/fixtures/catalog_server.py"), str(control)]}})["id"]
        entry = self.cli("gateway", "save", "--stdin", value={"connectionIds": [upstream]})
        unrelated = self.cli("gateway", "save", "--stdin", value={"connectionIds": []})
        aggregate = self.bridge(entry["id"], "refresh-aggregate")
        single = self.bridge(upstream, "refresh-single")
        other = self.bridge(unrelated["id"], "refresh-unrelated")
        for client in [aggregate, single]:
            for family in ["tools", "resources", "prompts"]:
                assert client.initialized["result"]["capabilities"][family]["listChanged"]
                assert "result" in client.call(family + "/list")
        self.passed("single and aggregate entries advertise list changes for their implemented catalogs")

        temporary = control.with_suffix(".tmp")
        temporary.write_text(json.dumps({"tools": 1, "resources": 1, "prompts": 1}))
        temporary.replace(control)
        for client in [aggregate, single]:
            for family in ["tools", "resources", "prompts"]:
                client.wait_notification(f"notifications/{family}/list_changed")
            assert any(t["name"].endswith("read_1") for t in client.call("tools/list")["result"]["tools"])
            assert any(r["uri"].endswith("/1") for r in client.call("resources/list")["result"]["resources"])
            assert any(p["name"].endswith("summary_1") for p in client.call("prompts/list")["result"]["prompts"])
        other.call("tools/list")
        assert not other.notifications
        self.passed("upstream tool/resource/prompt notifications reach affected entries and fresh lists without crossing entry scope")

        self.cli("gateway", "refresh", entry["id"])
        for family in ["tools", "resources", "prompts"]:
            aggregate.wait_notification(f"notifications/{family}/list_changed")
        single.call("tools/list")
        assert not single.notifications
        changed = self.cli("gateway", "save", "--stdin", value={"id": entry["id"], "editVersion": entry["editVersion"], "connectionIds": [upstream], "toolSelections": {upstream: []}})
        aggregate.wait_notification("notifications/tools/list_changed")
        assert aggregate.call("tools/list")["result"]["tools"] == []
        self.passed("manual refresh and policy edits notify the selected entry without rewriting native configuration")

        counts = json.loads(control.with_suffix(".counts.json").read_text())
        time.sleep(2.5)
        assert json.loads(control.with_suffix(".counts.json").read_text()) == counts
        assert counts["calls"] == 0
        self.cli("gateway", "stop")
        time.sleep(2.5)
        assert not (self.data / "runtime.json").exists(), "Idle catalog watchers restarted a stopped daemon"
        assert "result" in single.call("tools/list")
        aggregate.wait_notification("notifications/tools/list_changed")
        self.passed("idle watchers leave a stopped daemon stopped and detect its next request-driven restart")
        for client in [aggregate, single, other]:
            client.close()
            self.clients.remove(client)
            assert client.process.poll() == 0
        assert changed["members"][0]["includeTools"] == []
        self.passed("watchers do not rediscover idle catalogs or invoke business tools and exit with their MCP sessions")
        if self.pi_package:
            native = self.cli("gateway", "save", "--stdin", value={"connectionIds": [upstream]})
            source = self.cli("sources", "add", "--agent", "pi")
            plan = self.cli("changes", "preview", "--connection", native["id"], "--target", source["id"], "--mode", "shared")
            self.cli("changes", "apply", plan["planId"])
            project = self.record / "native-project"
            project.mkdir()
            agent_dir = Path(source["path"]).parent
            result = subprocess.run(["node", str(ROOT / "tests/fixtures/pi-catalog-refresh.mjs"), str(self.pi_package.resolve()), str(agent_dir), str(project), str(control)], env=self.environment, capture_output=True, text=True, timeout=45)
            assert result.returncode == 0, (result.stdout, result.stderr)
            (self.record / "native-pi.json").write_text(result.stdout + "\n")
            assert json.loads(control.with_suffix(".counts.json").read_text())["calls"] == 0
            self.passed("the official Pi client refreshes its live tool inventory from an upstream notification without a model or tool call")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--pi-package", type=Path)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = CatalogRefresh(record, args.pi_package)
    conclusion, error = "PASS", None
    try:
        harness.run()
    except Exception as exception:
        conclusion, error = "FAIL", str(exception)
        traceback.print_exc()
    finally:
        harness.close(False)
    (record / "report.json").write_text(json.dumps({"conclusion": conclusion, "error": error, "binary_sha256": hashlib.sha256(BINARY.read_bytes()).hexdigest(), "scenarios": harness.results, "commands": harness.commands}, indent=2) + "\n")
    print(conclusion, record, flush=True)
    raise SystemExit(conclusion != "PASS")


if __name__ == "__main__":
    main()
