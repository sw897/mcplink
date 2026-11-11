"""Exercise independent entry allowlists over one managed authorization."""
import argparse
import hashlib
import json
from pathlib import Path
import traceback
import urllib.request

from tool_calls import ToolCalls
from run import BINARY


class EntryPolicy(ToolCalls):
    def run(self):
        upstream = self.save("shared_account", url=self.url)
        login = self.cli("credentials", "login", upstream)
        assert urllib.request.urlopen(login["url"], timeout=15).status == 200
        original = self.cli("connections", "show", upstream)["connection"]
        registrations = self.stats()["registrations"]
        narrow = self.cli("gateway", "save", "--stdin", value={
            "connectionIds": [upstream], "toolSelections": {upstream: ["read_document"]},
        })
        broad = self.cli("gateway", "save", "--stdin", value={"connectionIds": [upstream]})
        name = lambda entry, tool: entry["members"][0]["namespace"] + "__" + tool
        a, b, single = self.bridge(narrow["id"], "narrow"), self.bridge(broad["id"], "broad"), self.bridge(upstream, "single")
        assert [t["name"] for t in a.call("tools/list")["result"]["tools"]] == [name(narrow, "read_document")]
        assert len(b.call("tools/list")["result"]["tools"]) == len(single.call("tools/list")["result"]["tools"]) == 2
        assert self.cli("connections", "show", upstream)["connection"]["revision"] == original["revision"]
        self.passed("two entries keep independent tool ranges without copying or mutating their shared service account")

        before = self.stats()["calls"]
        assert "error" in a.call("tools/call", {"name": name(narrow, "search_documents"), "arguments": {"query": "fixture"}})
        self.invoke(narrow["id"], name(narrow, "search_documents"), {"query": "fixture"}, exit_status=1)
        assert self.stats()["calls"] == before
        assert "result" in a.call("tools/call", {"name": name(narrow, "read_document"), "arguments": {"id": "weekly-fixture"}})
        assert "result" in b.call("tools/call", {"name": name(broad, "search_documents"), "arguments": {"query": "fixture"}})
        self.passed("forged hidden aliases are refused before dispatch while allowed tools reuse the same OAuth grant")
        generated = self.cli("cli", "generate", narrow["id"], "--name", "entry-read", "--output", str(self.record / "entry-cli"), "--tool", name(narrow, "read_document"))

        preserved = self.cli("gateway", "save", "--stdin", value={"id": narrow["id"], "editVersion": narrow["editVersion"], "title": "Renamed", "connectionIds": [upstream]})
        assert preserved["members"][0]["includeTools"] == ["read_document"]
        self.cli("gateway", "save", "--stdin", value={"id": narrow["id"], "editVersion": narrow["editVersion"], "connectionIds": [upstream]}, exit_status=1)
        empty = self.cli("gateway", "save", "--stdin", value={"id": narrow["id"], "editVersion": preserved["editVersion"], "connectionIds": [upstream], "toolSelections": {upstream: []}})
        assert a.call("tools/list")["result"]["tools"] == []
        assert "error" in a.call("tools/call", {"name": name(narrow, "read_document"), "arguments": {"id": "weekly-fixture"}})
        before = self.stats()["calls"]
        failure = self.cli("cli", "run", "--manifest", generated["manifest"], "--", name(narrow, "read_document"), "--stdin", value={"id": "weekly-fixture"}, exit_status=1)
        assert failure["error"]["code"] == "tool_not_found" and self.stats()["calls"] == before
        restored = self.cli("gateway", "save", "--stdin", value={"id": narrow["id"], "editVersion": empty["editVersion"], "connectionIds": [upstream], "toolSelections": {upstream: None}})
        assert len(a.call("tools/list")["result"]["tools"]) == 2
        assert self.cli("cli", "run", "--manifest", generated["manifest"], "--", name(narrow, "read_document"), "--stdin", value={"id": "weekly-fixture"})["output"]["isError"] is False
        self.passed("omitted policy retains it, empty denies all tools, null inherits and stale edits are refused")

        for selections in [{"missing": []}, {upstream: ["read_document", "read_document"]}, {upstream: "read_document"}, {upstream: [""]}, {upstream: ["bad\nname"]}]:
            self.cli("gateway", "save", "--stdin", value={"connectionIds": [upstream], "toolSelections": selections}, exit_status=1)
        self.passed("invalid, duplicate and unselected-member policies fail closed")

        definition = self.cli("connections", "show", upstream)["definition"]
        definition["includeTools"] = ["read_document"]
        self.cli("connections", "save", "--stdin", value={"id": upstream, "name": "shared_account", "definition": definition})
        assert len(b.call("tools/list")["result"]["tools"]) == 1
        assert "error" in b.call("tools/call", {"name": name(broad, "search_documents"), "arguments": {"query": "fixture"}})
        assert self.stats()["registrations"] == registrations == 1
        assert self.cli("connections", "show", restored["id"])["definition"] is None
        self.passed("the source-wide policy remains the upper bound and all views keep one authorization")


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    record = parser.parse_args().record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    harness = EntryPolicy(record)
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
