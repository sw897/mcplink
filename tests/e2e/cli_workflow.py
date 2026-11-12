"""Desktop management and named CLI share generation, contract checks and calls."""
import argparse
import hashlib
import json
from pathlib import Path
import socket
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
        assert checked["discovery"] == {"status": "complete", "diagnostics": []}
        before = self.stats()["calls"]
        assert self.launch(manifest, "read_document", "--id", "weekly-fixture")["output"]["isError"] is False
        assert self.stats()["calls"] == before + 1
        self.passed("desktop RPC and CLI generate, inspect, check and execute the same portable manifest")

        original = manifest.read_bytes()
        assert self.rpc("cli.generate", {"id": connection, "name": "workflow-tools", "output": str(output), "tools": ["read_document"]}, error=True)
        assert manifest.read_bytes() == original
        self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-generated-changed")
        checked = self.rpc("cli.check", {"manifest": str(manifest)})
        assert not checked["compatible"] and checked["tools"][0]["status"] == "changed"
        assert checked["tools"][0]["changes"]["parameters"] == [
            {"name": "newOptionalFlag", "change": "added", "after": {"name": "newOptionalFlag", "kind": "boolean", "required": False}}]
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

        before = self.stats()["calls"]
        for suffix, verify in [
            ("contract-v2", lambda c: {p["name"]: p["change"] for p in c["parameters"]} == {"delay_ms": "removed", "id": "modified", "revision": "added"}),
            ("constraints", lambda c: c["schemaChanged"] and not c["parameters"] and c["stdinOnly"] is None),
            ("stdin", lambda c: c["stdinOnly"] == {"before": False, "after": True} and not c["parameters"]),
        ]:
            self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-generated-{suffix}")
            result = self.cli("cli", "check", str(manifest))
            assert result["discovery"]["status"] == "complete" and result["tools"][0]["status"] == "changed", result
            assert verify(result["tools"][0]["changes"]), result
        self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-generated-invalid")
        result = self.cli("cli", "check", str(manifest))
        assert result["tools"][0]["status"] == "changed" and result["tools"][0]["reason"]["code"] == "invalid_tool_schema", result
        assert manifest.read_bytes() == original and self.stats()["calls"] == before
        self.passed("parameter additions, removals, kinds, required flags, stdin mode and opaque schema constraints have credential-free change summaries")

        def unchecked(code, path=manifest):
            result = self.cli("cli", "check", str(path))
            assert not result["compatible"] and result["discovery"]["status"] == "failed", result
            assert result["discovery"]["diagnostics"][0]["code"] == code, result
            assert result["tools"][0]["status"] == "unchecked" and result["tools"][0]["reason"]["code"] == code, result
            return result

        self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-generated")
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-invalid-authorization"})
        unchecked("authentication_required")
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-fixture-authorization"})
        self.cli("gateway", "pause")
        unchecked("gateway_paused")
        self.cli("gateway", "resume")
        self.cli("credentials", "disconnect", connection)
        unchecked("connection_inactive")
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-fixture-authorization"})
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            unavailable_port = listener.getsockname()[1]
        offline_connection = self.save("workflow_offline", url=f"http://127.0.0.1:{unavailable_port}/mcp")
        offline_manifest = self.record / "offline.mcplink.json"
        offline_manifest.write_text(json.dumps(json.loads(original) | {"connectionId": offline_connection}))
        unchecked("transport_failed", offline_manifest)
        self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-cycle")
        unchecked("tool_discovery_failed")
        self.definition(connection, url=f"http://127.0.0.1:{self.port}/mcp-generated", includeTools=["search_documents"], includeSpecified=True)
        result = self.cli("cli", "check", str(manifest))
        assert result["discovery"]["status"] == "complete" and result["tools"][0]["status"] == "unavailable", result
        assert result["tools"][0]["reason"]["code"] == "tool_unavailable", result
        self.definition(connection, includeTools=[], includeSpecified=False)
        assert self.cli("cli", "check", str(manifest))["compatible"]
        assert manifest.read_bytes() == original and self.stats()["calls"] == before
        self.passed("auth, transport, paused, inactive and invalid discovery are unchecked; only a complete catalog confirms a removed or filtered tool")

        other = self.save("workflow_other", url=f"http://127.0.0.1:{self.port}/mcp-generated")
        entry = self.rpc("gateway.save", {"title": "Workflow entry", "connectionIds": [connection, other]})
        aliases = [member["namespace"] + "__read_document" for member in entry["members"]]
        generated = self.rpc("cli.generate", {"id": entry["id"], "name": "entry-tools", "output": str(self.record / "entry-tools"), "tools": aliases})
        entry_manifest = Path(generated["manifest"])
        self.generated.append(entry_manifest.parent)
        entry_bytes = entry_manifest.read_bytes()
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-invalid-authorization"})
        result = self.cli("cli", "check", str(entry_manifest))
        assert result["discovery"]["status"] == "partial" and not result["compatible"], result
        assert [t["status"] for t in result["tools"]] == ["unchecked", "current"], result
        assert result["tools"][0]["reason"]["code"] == "authentication_required", result
        assert result["discovery"]["diagnostics"][0]["connectionId"] == connection, result
        # An unrelated failed member must not block a CLI containing only healthy tools.
        healthy_manifest = json.loads(entry_bytes)
        healthy_manifest["tools"] = healthy_manifest["tools"][1:]
        healthy_path = self.record / "healthy.mcplink.json"
        healthy_path.write_text(json.dumps(healthy_manifest))
        result = self.cli("cli", "check", str(healthy_path))
        assert result["compatible"] and result["discovery"]["status"] == "partial", result
        self.cli("credentials", "update", connection, "--stdin", value={"value": "synthetic-fixture-authorization"})
        assert self.cli("cli", "check", str(entry_manifest))["compatible"]
        foreign = self.record / "other-dataset"
        self.extra_data.append(foreign)
        result = self.cli("--data-dir", str(foreign), "cli", "check", str(entry_manifest))
        assert result["discovery"]["status"] == "not_checked" and not result["datasetMatches"], result
        assert all(t["status"] == "unchecked" and t["reason"]["code"] == "dataset_mismatch" for t in result["tools"]), result
        assert entry_manifest.read_bytes() == entry_bytes and self.stats()["calls"] == before
        for item in (result, self.cli("cli", "check", str(manifest))):
            serialized = json.dumps(item)
            assert not any(secret in serialized for secret in ["synthetic-fixture-authorization", "synthetic-invalid-authorization", "127.0.0.1", "inputSchema", "synthetic-schema-"]), serialized
        self.passed("partial aggregate discovery keeps healthy contracts, attributes failed members and never calls tools, rewrites manifests or guesses another dataset")


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
