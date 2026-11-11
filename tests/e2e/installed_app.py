"""Install from a DMG and exercise public CLI/Bridge recovery in an isolated home."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import time

import run as e2e


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--dmg", type=Path)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--previous-app", type=Path)
    parser.add_argument("--previous-dmg", type=Path, default=os.environ.get("MCPLINK_PREVIOUS_DMG"))
    parser.add_argument("--keep", action="store_true")
    args = parser.parse_args()
    version = json.loads((e2e.ROOT / "package.json").read_text())["version"]
    arch = "aarch64" if platform.machine() == "arm64" else "x64"
    dmg = args.dmg or e2e.ROOT / f"target/release/bundle/dmg/mcplink_{version}_{arch}.dmg"
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    report = {"executed_at": datetime.now(timezone.utc).isoformat(), "platform": platform.platform(),
              "scope": "DMG copy installation, isolated native configs and fixture Vault; same macOS user",
              "scenarios": []}
    mount, harness = None, None

    def passed(name):
        report["scenarios"].append({"scenario": name, "result": "PASS"})
        print("PASS", name, flush=True)

    def use_app(app):
        e2e.BINARY = app / "Contents/MacOS/mcplink"
        return e2e.BINARY

    def stop_daemon():
        runtime = json.loads((harness.data / "runtime.json").read_text())
        subprocess.run(["kill", "-TERM", str(runtime["pid"])], check=True)
        # Wait for this fixture's runtime lock, rather than relying on a fixed delay.
        import fcntl
        with (harness.data / "runtime.lock").open("rb") as lock:
            for _ in range(100):
                try:
                    fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                    fcntl.flock(lock, fcntl.LOCK_UN)
                    return
                except BlockingIOError:
                    time.sleep(0.05)
        raise AssertionError("Fixture daemon did not stop")

    try:
        assert sys.platform == "darwin", "DMG lifecycle requires macOS"
        assert not (args.previous_app and args.previous_dmg), "Select one previous App or DMG"
        if args.previous_dmg:
            previous_mount = None
            try:
                attached = subprocess.check_output(["hdiutil", "attach", "-readonly", "-nobrowse", "-mountrandom", "/Volumes", "-plist", str(args.previous_dmg.resolve())])
                previous_mount = next(Path(e["mount-point"]) for e in plistlib.loads(attached)["system-entities"] if "mount-point" in e)
                args.previous_app = record / "Previous Build/mcplink.app"
                args.previous_app.parent.mkdir()
                subprocess.run(["ditto", str(previous_mount / "mcplink.app"), str(args.previous_app)], check=True)
            finally:
                if previous_mount:
                    subprocess.run(["hdiutil", "detach", str(previous_mount)], check=True, capture_output=True)
        attached = subprocess.check_output(["hdiutil", "attach", "-readonly", "-nobrowse", "-mountrandom", "/Volumes", "-plist", str(dmg.resolve())])
        mount = next(Path(e["mount-point"]) for e in plistlib.loads(attached)["system-entities"] if "mount-point" in e)
        application = record / "Test Applications/mcplink.app"
        application.parent.mkdir()
        subprocess.run(["ditto", str(mount / "mcplink.app"), str(application)], check=True)
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(application)], check=True)
        use_app(application)
        fixture = record / "fixture"
        fixture.mkdir()
        harness = e2e.Harness(fixture)
        harness.environment["PATH"] = "/usr/bin:/bin:/usr/sbin:/sbin"
        # Do not inherit a live Claude profile while checking newly added sources.
        harness.environment["CLAUDE_CONFIG_DIR"] = str(harness.home / ".claude")
        if args.previous_app:
            # Initialize the older release first; never attempt to downgrade new data.
            saved = record / "Current Build/mcplink.app"
            saved.parent.mkdir()
            shutil.move(application, saved)
            subprocess.run(["ditto", str(args.previous_app.resolve()), str(application)], check=True)
            use_app(application)
            harness.rpc("snapshot")
            old_connection = harness.rpc("connections.import", {"sourceId":"hermes:user", "name":"office_docs"})["id"]
            assert harness.rpc("connections.probe", {"id":old_connection})["connection"]["status"] == "ready"
            harness.changes(old_connection, ["codex:user", "hermes:user"], mode="shared")
            configs = [harness.home / ".codex/config.toml", harness.home / ".hermes/config.yaml"]
            before = [p.read_bytes() for p in configs]
            stop_daemon()
            shutil.rmtree(application)
            subprocess.run(["ditto", str(saved), str(application)], check=True)
            use_app(application)
            assert harness.rpc("connections.probe", {"id":old_connection})["connection"]["status"] == "ready"
            assert [p.read_bytes() for p in configs] == before
            report["previous_binary_sha256"] = hashlib.sha256((args.previous_app / "Contents/MacOS/mcplink").read_bytes()).hexdigest()
            passed("installing over an older initialized build retains account IDs, credentials and native configurations")
            stop_daemon()
            import sqlite3
            with sqlite3.connect(harness.data / "mcplink.sqlite3") as database:
                schema = database.execute("PRAGMA user_version").fetchone()[0]
            report["upgraded_database_semantics"] = schema
            # Record, rather than assume, how the actual previous executable treats it.
            previous = subprocess.Popen([str(args.previous_app / "Contents/MacOS/mcplink"), "daemon", "--data-dir", str(harness.data)], env=harness.environment, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            try:
                stdout, stderr = previous.communicate(timeout=2)
                report["previous_backend_refused_new_data"] = previous.returncode != 0 and "数据库来自较新版本" in stdout + stderr
            except subprocess.TimeoutExpired:
                # Same-semantic-version baselines remain usable; stop only this fixture.
                stop_daemon()
                previous.communicate(timeout=10)
        initial = harness.rpc("snapshot")
        assert {s["agent"] for s in initial["sources"]} >= {"codex", "hermes"}
        passed("DMG installation under a path with spaces works with minimal Finder PATH")
        connection = harness.rpc("connections.import", {"sourceId": "hermes:user", "name": "office_docs"})["id"]
        assert harness.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
        harness.changes(connection, ["codex:user", "hermes:user"], mode="shared")
        a, b = harness.bridge(connection, "installed-codex"), harness.bridge(connection, "installed-hermes")
        for client in (a, b):
            assert len(client.call("tools/list")["result"]["tools"]) == 2
        passed("installed CLI binds two native config sources and serves two live Bridges")
        harness.rpc("credentials.update", {"id": connection, "value": "synthetic-invalid-authorization"})
        for client in (a, b):
            assert "error" in client.call("tools/list"), "Old credentials unexpectedly work"
        harness.rpc("credentials.update", {"id": connection, "value": "synthetic-fixture-authorization"})
        for client in (a, b):
            assert "result" in client.call("tools/list")
        passed("one credential update recovers both existing Bridges")
        stop_daemon()
        for client in (a, b):
            assert "result" in client.call("tools/list")
        passed("existing Bridges restart the background process and recover")
        for client in harness.clients:
            client.close()
        harness.clients.clear()
        stop_daemon()
        moved = record / "Moved Applications/mcplink.app"
        moved.parent.mkdir()
        shutil.move(application, moved)
        use_app(moved)
        assert harness.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
        plan, _ = harness.changes(connection, ["codex:user", "hermes:user"], mode="shared")
        assert all(c["after"]["command"] == str(e2e.BINARY) for c in plan["changes"])
        with_new_path = harness.bridge(connection, "reloaded-after-move")
        assert "result" in with_new_path.call("tools/list")
        passed("moving the app and reapplying preview repairs native shared paths")
        report.update(conclusion="PASS", binary=str(e2e.BINARY),
                      binary_sha256=hashlib.sha256(e2e.BINARY.read_bytes()).hexdigest(),
                      native_user_config_modified=False,
                      limits=["No new macOS account, real Keychain, quarantine or Windows acceptance",
                              "Menu bar, login startup, Dock reopening and desktop quit require separate native UI acceptance",
                              "Previous-build compatibility is not an upgrade from a released older version"])
        if args.keep:
            # Used only by the desktop QA copy; contains synthetic paths/credentials.
            keys = ("MCPLINK_HOME", "MCPLINK_DATA_DIR", "MCPLINK_VAULT", "CODEX_HOME", "HERMES_HOME", "CLAUDE_CONFIG_DIR", "PATH")
            (record / "fixture-environment.json").write_text(json.dumps({k: harness.environment[k] for k in keys}, indent=2) + "\n")
    except Exception as error:
        report.update(conclusion="FAIL", error=str(error))
    finally:
        if mount:
            detached = subprocess.run(["hdiutil", "detach", str(mount)], capture_output=True, text=True)
            if detached.returncode:
                report.update(conclusion="FAIL", detach_error=detached.stderr.strip())
        if harness and (not args.keep or report.get("conclusion") != "PASS"):
            harness.close(False)
        (record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(report["conclusion"], report.get("error", ""), flush=True)
    if args.keep and report["conclusion"] == "PASS":
        try:
            while True:
                time.sleep(30)
        except KeyboardInterrupt:
            harness.close(False)
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
