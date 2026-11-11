"""Desktop management and named CLI share generation, contract checks and calls."""
import argparse
import hashlib
import json
from pathlib import Path
import traceback

from generated_cli import GeneratedCli
from run import BINARY


class CliWorkflow(GeneratedCli):
    def run(self):
        connection = self.save("workflow", url=f"http://127.0.0.1:{self.port}/mcp-generated")
        output = self.record / "desktop-generated"
        generated = self.rpc("cli.generate", {"id": connection, "name": "workflow-tools", "output": str(output), "tools": ["read_document"]})
        manifest = Path(generated["manifest"])
        self.generated.append(output)
        inspected = self.rpc("cli.inspect", {"manifest": str(manifest)})
        assert inspected == self.cli("cli", "inspect", str(manifest))
        checked = self.cli("cli", "check", str(manifest))
        assert checked["compatible"] and checked["tools"] == [{"name": "read_document", "status": "current"}]
        before = self.stats()["calls"]
        assert self.launch(manifest, "read_document", "--id", "weekly-fixture")["output"]["isError"] is False
        assert self.stats()["calls"] == before + 1
        self.passed("desktop RPC and CLI generate, inspect, check and execute the same portable manifest")

        original = manifest.read_bytes()
        assert self.rpc("cli.generate", {"id": connection, "name": "workflow-tools", "output": str(output), "tools": ["read_document"]}, error=True)
        assert manifest.read_bytes() == original
        self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-generated-changed")
        checked = self.rpc("cli.check", {"manifest": str(manifest)})
        assert not checked["compatible"] and checked["tools"] == [{"name": "read_document", "status": "changed"}]
        before = self.stats()["calls"]
        failure = self.launch(manifest, "read_document", "--id", "weekly-fixture", exit_status=1)
        assert failure["error"]["code"] == "tool_schema_changed" and self.stats()["calls"] == before
        assert manifest.read_bytes() == original
        self.passed("contract changes are reported and refuse stale execution without overwriting or calling the tool")

        regenerated = self.rpc("cli.generate", {"id": connection, "name": "workflow-tools", "output": str(self.record / "reviewed-v2"), "tools": ["read_document"]})
        fresh = Path(regenerated["manifest"])
        self.generated.append(fresh.parent)
        assert self.cli("cli", "check", str(fresh))["compatible"]
        assert self.launch(fresh, "read_document", "--stdin", value={"id": "weekly-fixture", "revision": 2})["output"]["isError"] is False
        self.passed("reviewed regeneration uses a new directory while preserving the prior CLI")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = CliWorkflow(record)
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
