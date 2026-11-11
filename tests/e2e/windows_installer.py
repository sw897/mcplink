"""Exercise NSIS install, upgrade, reinstall and uninstall in an isolated fixture."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time
import traceback
import urllib.request

import run as e2e


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--installer", type=Path, required=True)
    parser.add_argument("--previous-installer", type=Path)
    parser.add_argument("--require-signature", action="store_true")
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    report = {"conclusion": "FAIL", "scenarios": [], "scope": "isolated NSIS installation and synthetic credential store"}
    original_binary, harness = e2e.BINARY, None

    def passed(name):
        report["scenarios"].append({"scenario": name, "result": "PASS"})
        print("PASS", name, flush=True)

    def install(installer, directory):
        # NSIS requires /D= to be the last argument and unquoted, even with spaces.
        assert '"' not in str(directory) and not any(c in str(directory) for c in '\r\n')
        command = subprocess.list2cmdline([str(installer.resolve()), "/S"]) + " /D=" + str(directory)
        result = subprocess.run(command, env=harness.environment, capture_output=True, text=True, timeout=180)
        assert result.returncode == 0, "NSIS installation failed"
        deadline = time.monotonic() + 20
        while not (directory / "mcplink.exe").is_file():
            assert time.monotonic() < deadline, "Installed CLI is missing"
            time.sleep(0.1)

    def stop():
        result = subprocess.run([str(e2e.BINARY), "gateway", "stop"], env=harness.environment, capture_output=True, text=True, timeout=90)
        assert result.returncode == 0, "Installed daemon did not release its runtime lock"

    try:
        assert os.name == "nt", "NSIS lifecycle requires Windows"
        fixture = record / "fixture"
        fixture.mkdir()
        harness = e2e.Harness(fixture)
        directory = record / "Test Applications/mcplink"
        first = args.previous_installer or args.installer
        install(first, directory)
        e2e.BINARY = directory / "mcplink.exe"
        report["previous_installer_sha256"] = hashlib.sha256(first.read_bytes()).hexdigest()
        report["distinct_previous_installer"] = args.previous_installer is not None and first.read_bytes() != args.installer.read_bytes()
        token = harness.rpc("connections.import", {"sourceId": "hermes:user", "name": "office_docs"})["id"]
        assert harness.rpc("connections.probe", {"id": token})["connection"]["status"] == "ready"
        oauth = harness.rpc("connections.save", {"name": "upgrade_oauth", "definition": {"url": harness.url, "oauth": True}})["id"]
        login = harness.rpc("oauth.start", {"id": oauth})
        assert urllib.request.urlopen(login["url"], timeout=15).status == 200
        assert harness.rpc("connections.probe", {"id": oauth})["connection"]["status"] == "ready"
        harness.changes(token, ["codex:user", "hermes:user"], mode="shared")
        configs = [harness.home / ".codex/config.toml", harness.home / ".hermes/config.yaml"]
        before = [p.read_bytes() for p in configs]
        stop()
        passed("the baseline installer initializes token and OAuth accounts and two native shared bindings")

        install(args.installer, directory)
        for connection in [token, oauth]:
            assert harness.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
        assert [p.read_bytes() for p in configs] == before
        client = harness.bridge(token, "installed-upgrade")
        assert "result" in client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
        client.close(); harness.clients.clear()
        stop()
        passed("installing the candidate preserves managed identities, authentication and native bytes and supports an actual MCP call")
        if args.require_signature:
            for path in [args.installer.resolve(), directory / "mcplink.exe", directory / "mcplink-desktop.exe"]:
                subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(e2e.ROOT / "scripts/sign-windows.ps1"), "-VerifyOnly", "-FilePath", str(path)], check=True)
            passed("installer and installed desktop/CLI signatures match the selected publisher and include timestamps")
        install(args.installer, directory)
        assert [p.read_bytes() for p in configs] == before
        assert harness.rpc("connections.probe", {"id": oauth})["connection"]["status"] == "ready"
        stop()
        passed("reinstalling the candidate retains OAuth authorization and native configuration")
        uninstaller = next(directory.glob("uninstall*.exe"))
        result = subprocess.run([str(uninstaller), "/S"], env=harness.environment, capture_output=True, text=True, timeout=120)
        assert result.returncode == 0, "Uninstall failed"
        deadline = time.monotonic() + 30
        while (directory / "mcplink.exe").exists():
            assert time.monotonic() < deadline, "Uninstall left executable files"
            time.sleep(0.1)
        e2e.BINARY = original_binary
        assert [p.read_bytes() for p in configs] == before
        assert {c["id"] for c in harness.rpc("snapshot")["connections"]} >= {token, oauth}
        passed("uninstall removes binaries while preserving managed data and native configuration for recovery")
        report.update(conclusion="PASS", installer_sha256=hashlib.sha256(args.installer.read_bytes()).hexdigest(), real_user_configuration_modified=False)
        if not args.previous_installer:
            report["limits"] = ["No previous installer supplied: install/reinstall/uninstall only; not a cross-version upgrade"]
    except Exception as error:
        report["error"] = str(error)
        traceback.print_exc()
    finally:
        e2e.BINARY = original_binary
        if harness:
            harness.close(False)
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(report["conclusion"], record, flush=True)
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
