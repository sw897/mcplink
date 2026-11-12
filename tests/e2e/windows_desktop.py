"""Verify an installed Windows Tauri/WebView2 window through native UI Automation.

This is separate from the Chrome renderer test. Missing interactive desktop or
accessibility support is a failed acceptance, never a simulated success.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import traceback

import run as e2e
from http_api import ApiServer, tool


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--installer", type=Path, required=True)
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    assert os.name == "nt", "Native UI Automation requires Windows"
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    fixture = record / "fixture"
    fixture.mkdir()
    harness = e2e.Harness(fixture)
    api = ApiServer()
    directory = record / "Installed Applications/mcplink"
    process = None
    original_binary = e2e.BINARY
    report = {"conclusion": "FAIL", "scope": "installed Tauri and WebView2; native Windows UI Automation; synthetic Vault and upstream"}
    try:
        command = subprocess.list2cmdline([str(args.installer.resolve()), "/S"]) + " /D=" + str(directory)
        installed = subprocess.run(command, env=harness.environment, capture_output=True, timeout=180)
        assert installed.returncode == 0, "NSIS installation failed"
        desktop = directory / "mcplink-desktop.exe"
        e2e.BINARY = directory / "mcplink.exe"
        assert desktop.is_file() and e2e.BINARY.is_file(), "Installed native application missing"
        report["desktop_sha256"] = hashlib.sha256(desktop.read_bytes()).hexdigest()
        connection = harness.rpc("connections.save", {"name": "windows_api", "title": "Windows API 验收", "definition": {
            "url": api.endpoint, "headers": {"Authorization": "Bearer fixture-http-api"}, "httpApi": {
                "tools": [tool()], "probe": {"tool": "echo", "arguments": {"id": "native-desktop", "count": 1, "enabled": False, "items": [1]}}}}})["id"]
        environment = harness.environment | {"MCPLINK_CLI": str(e2e.BINARY),
            "WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS": "--force-renderer-accessibility",
            "WEBVIEW2_USER_DATA_FOLDER": str(record / "webview2")}
        process = subprocess.Popen([str(desktop)], env=environment, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        # Windows PowerShell 5 reads UTF-8 source correctly only with a BOM.
        native_script = record / "native-ui.ps1"
        native_script.write_text((e2e.ROOT / "tests/fixtures/windows-desktop.ps1").read_text(encoding="utf-8"), encoding="utf-8-sig")
        with (record / "native-ui.log").open("w", encoding="utf-8") as log:
            result = subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                str(native_script), "-DesktopProcess", str(process.pid),
                "-DesktopPath", str(desktop)], env=environment, stdout=log, stderr=subprocess.STDOUT, timeout=240)
        # Only these fixed assertions enter public logs. The UI dump/log is private.
        phases = [line.strip() for line in (record / "native-ui.log").read_text(errors="replace").splitlines()
                  if line.strip() in {"NATIVE_STEP " + step for step in ["window", "navigation", "http-probe", "background", "tray-reopen"]}]
        print("NATIVE_DESKTOP " + json.dumps({"phases": phases, "exit_status": result.returncode}), flush=True)
        assert result.returncode == 0, "Native Windows desktop interaction failed; see private native-ui.log"
        assert len(api.requests) == 1, "Opening/cancelling or confirmation dispatched an unexpected number of API requests"
        assert harness.rpc("connections.detail", {"id": connection})["connection"]["status"] == "ready"
        assert not harness.rpc("gateway.status")["paused"]
        assert process.poll() is None, "Native desktop exited during close-to-tray test"
        report["conclusion"] = "PASS"
        report["checks"] = ["installed native window", "navigation and modal cancellation", "HTTP probe exactly once",
                            "pause/resume through native RPC", "close-to-tray", "single-instance reopen"]
        print("PASS Windows installed native window, modal, HTTP probe, background controls and tray reopen", flush=True)
    except Exception:
        traceback.print_exc()
    finally:
        if process and process.poll() is None:
            process.terminate()
            process.wait(timeout=15)
        harness.close(False)
        uninstaller = next(directory.glob("uninstall*.exe"), None)
        if uninstaller:
            subprocess.run([str(uninstaller), "/S"], env=harness.environment, capture_output=True, timeout=120)
        e2e.BINARY = original_binary
        api.close()
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
