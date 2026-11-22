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
from daemon_lifecycle import exited, wait_until
from release_vault import verify as verify_release_vault


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--installer", type=Path, required=True)
    parser.add_argument("--previous-installer", type=Path)
    parser.add_argument("--require-signature", action="store_true")
    modes = parser.add_mutually_exclusive_group()
    modes.add_argument("--production", action="store_true", help="Install the distributable and verify the system Vault boundary")
    modes.add_argument("--native-migration", action="store_true", help="Upgrade distinct distributables using synthetic grants in Windows Credential Manager")
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    if args.production and args.previous_installer:
        parser.error("The production package smoke test uses a clean install")
    if args.native_migration and not args.previous_installer:
        parser.error("--native-migration requires a previous distributable installer")
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    report = {"conclusion": "FAIL", "scenarios": [], "scope": "isolated NSIS installation and synthetic credential store"}
    original_binary, harness, native = e2e.BINARY, None, None

    def passed(name):
        report["scenarios"].append({"scenario": name, "result": "PASS"})
        print("PASS", name, flush=True)

    def install(installer, directory, phase):
        # NSIS requires /D= to be the last argument and unquoted, even with spaces.
        assert '"' not in str(directory) and not any(c in str(directory) for c in '\r\n')
        command = subprocess.list2cmdline([str(installer.resolve()), "/S"]) + " /D=" + str(directory)
        result = subprocess.run(command, env=harness.environment, capture_output=True, text=True, timeout=180)
        assert result.returncode == 0, f"NSIS {phase} failed (exit code {result.returncode})"
        deadline = time.monotonic() + 20
        while not (directory / "mcplink.exe").is_file():
            assert time.monotonic() < deadline, "Installed CLI is missing"
            time.sleep(0.1)

    def stop():
        pid = json.loads((harness.data / "runtime.json").read_text())["pid"]
        result = subprocess.run([str(e2e.BINARY), "gateway", "stop"], env=harness.environment, capture_output=True, text=True, timeout=90)
        assert result.returncode == 0, "Installed daemon did not release its runtime lock"
        # NSIS must be able to replace the CLI executable after the runtime lock is released.
        if not exited(pid):
            print("WAIT for the installed daemon process to exit before running NSIS", flush=True)
            wait_until(lambda: exited(pid))

    def verify_signature(directory):
        if args.require_signature:
            for path in [args.installer.resolve(), directory / "mcplink.exe", directory / "mcplink-desktop.exe"]:
                subprocess.run(["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(e2e.ROOT / "scripts/sign-windows.ps1"), "-VerifyOnly", "-FilePath", str(path)], check=True)
            passed("installer and installed desktop/CLI signatures match the selected publisher and include timestamps")

    def management_api():
        runtime = json.loads((harness.data / "runtime.json").read_text())
        with urllib.request.urlopen(f'http://127.0.0.1:{runtime["port"]}/health', timeout=10) as response:
            return json.load(response)["managementApi"]

    def try_generated_cli(launcher):
        process = subprocess.run(
            ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
             str(launcher), "read_document", "--stdin"],
            env=harness.environment | {"MCPLINK_BIN": str(e2e.BINARY)},
            input=json.dumps({"id": "weekly-fixture"}), capture_output=True, encoding="utf-8", timeout=45,
        )
        assert process.returncode == 0, "The existing generated CLI failed to invoke a synthetic tool"
        assert json.loads(process.stdout)["ok"]["output"]["isError"] is False

    def uninstall(directory):
        uninstaller = next(directory.glob("uninstall*.exe"))
        result = subprocess.run([str(uninstaller), "/S"], env=harness.environment, capture_output=True, text=True, timeout=120)
        assert result.returncode == 0, f"NSIS uninstall failed (exit code {result.returncode})"
        deadline = time.monotonic() + 30
        while (directory / "mcplink.exe").exists():
            assert time.monotonic() < deadline, "Uninstall left executable files"
            time.sleep(0.1)

    def exercise():
        nonlocal harness, native
        assert os.name == "nt", "NSIS lifecycle requires Windows"
        fixture = record / "fixture"
        fixture.mkdir()
        harness = e2e.Harness(fixture)
        if args.native_migration:
            from windows_native_storage import NativeStorage
            native = NativeStorage(harness)
            report["scope"] = "isolated NSIS upgrade, native NTFS ACL and Windows Credential Manager with synthetic data"
        directory = record / "Test Applications/mcplink"
        first = args.previous_installer or args.installer
        install(first, directory, "baseline install")
        if args.production:
            report["scope"] = "isolated NSIS installation of the distributable package"
            cli = directory / "mcplink.exe"
            schema = json.loads(subprocess.check_output([str(cli), "schema"], encoding="utf-8"))["ok"]
            assert schema["version"] == json.loads((e2e.ROOT / "package.json").read_text())["version"]
            assert (directory / "mcplink-desktop.exe").is_file()
            verify_release_vault(cli, record / "release-vault")
            passed("installed release CLI identifies itself and refuses fixture Vault")
            verify_signature(directory)
            uninstall(directory)
            passed("distributable package installs and uninstalls in the isolated directory")
            return
        e2e.BINARY = directory / "mcplink.exe"
        report["previous_installer_sha256"] = hashlib.sha256(first.read_bytes()).hexdigest()
        report["distinct_previous_installer"] = args.previous_installer is not None and first.read_bytes() != args.installer.read_bytes()
        if native:
            assert report["distinct_previous_installer"], "The native migration must use distinct installers"
            report["previous_binary_sha256"] = hashlib.sha256(e2e.BINARY.read_bytes()).hexdigest()
        token = harness.rpc("connections.import", {"sourceId": "hermes:user", "name": "office_docs"})["id"]
        assert harness.rpc("connections.probe", {"id": token})["connection"]["status"] == "ready"
        oauth = harness.rpc("connections.save", {"name": "upgrade_oauth", "definition": {"url": harness.url, "oauth": True}})["id"]
        login = harness.rpc("oauth.start", {"id": oauth})
        assert urllib.request.urlopen(login["url"], timeout=15).status == 200
        assert harness.rpc("connections.probe", {"id": oauth})["connection"]["status"] == "ready"
        harness.changes(token, ["codex:user", "hermes:user"], mode="shared")
        configs = [harness.home / ".codex/config.toml", harness.home / ".hermes/config.yaml"]
        if native:
            report["previous_management_api"] = management_api()
            output = record / "Existing generated CLI"
            subprocess.run([str(e2e.BINARY), "cli", "generate", token, "--name", "upgrade-tools",
                            "--output", str(output), "--tool", "read_document"],
                           env=harness.environment, check=True, capture_output=True, timeout=45)
            launcher = output / "upgrade-tools.ps1"
            existing_launcher = launcher.read_bytes()
            try_generated_cli(launcher)
            native.prepare(oauth)
        before = [p.read_bytes() for p in configs]
        stop()
        passed("the baseline installer initializes token and OAuth accounts and two native shared bindings")

        install(args.installer, directory, "upgrade")
        if native:
            report["installed_binary_sha256"] = hashlib.sha256(e2e.BINARY.read_bytes()).hexdigest()
            assert report["installed_binary_sha256"] != report["previous_binary_sha256"], "Installer did not replace the CLI"
            assert report["installed_binary_sha256"] == hashlib.sha256(original_binary.read_bytes()).hexdigest(), "Installed CLI differs from the acceptance build"
        for connection in [token, oauth]:
            assert harness.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
        assert [p.read_bytes() for p in configs] == before
        client = harness.bridge(token, "installed-upgrade")
        assert "result" in client.call("tools/call", {"name": "read_document", "arguments": {"id": "weekly-fixture"}})
        client.close(); harness.clients.clear()
        if native:
            report["management_api"] = management_api()
            native.assert_oauth()
            assert launcher.read_bytes() == existing_launcher
            try_generated_cli(launcher)
        passed("installing the candidate preserves managed identities, authentication and native bytes and supports an actual MCP call")
        if native:
            native.verify_migration(passed)
            before = [p.read_bytes() for p in configs]
        stop()
        verify_signature(directory)
        install(args.installer, directory, "reinstall")
        assert [p.read_bytes() for p in configs] == before
        assert harness.rpc("connections.probe", {"id": oauth})["connection"]["status"] == "ready"
        stop()
        passed("reinstalling the candidate retains OAuth authorization and native configuration")
        if native:
            assert hashlib.sha256(e2e.BINARY.read_bytes()).hexdigest() == report["installed_binary_sha256"]
        uninstall(directory)
        e2e.BINARY = original_binary
        assert [p.read_bytes() for p in configs] == before
        assert {c["id"] for c in harness.rpc("snapshot")["connections"]} >= {token, oauth}
        if native:
            for connection in [token, oauth]:
                assert harness.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
            native.assert_oauth()
            stop()
        passed("uninstall removes binaries while preserving managed data and native configuration for recovery")

    try:
        exercise()
        report.update(conclusion="PASS", installer_sha256=hashlib.sha256(args.installer.read_bytes()).hexdigest(), real_user_configuration_modified=False)
        if not args.production and not args.previous_installer:
            report["limits"] = ["No previous installer supplied: install/reinstall/uninstall only; not a cross-version upgrade"]
    except Exception as error:
        report["error"] = str(error)
        traceback.print_exc()
    finally:
        e2e.BINARY = original_binary
        try:
            if harness:
                harness.close(False)
            if native:
                native.close()
                report["synthetic_credentials_removed"] = True
        except Exception:
            report["conclusion"] = "FAIL"
            report["cleanup_failed"] = True
            traceback.print_exc()
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(report["conclusion"], record, flush=True)
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
