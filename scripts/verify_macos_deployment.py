"""Snapshot or verify a real macOS deployment without exporting credentials."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import plistlib
import shlex
import subprocess
import sys
import time
import urllib.request

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest() if path.is_file() else None


def snapshot(cli, data, no_ui):
    prefix = [str(cli), "--data-dir", str(data)] + (["--no-auth-ui"] if no_ui else [])
    def call(*args):
        process = subprocess.run([*prefix, *args], capture_output=True, text=True, timeout=20)
        if process.returncode:
            raise RuntimeError("Noninteractive credential/management check failed; authorize through the trusted system UI, then retry")
        return json.loads(process.stdout)["ok"]
    status = call("status")
    gateways = call("gateway", "list")
    fields = ("agent", "sourceId", "connectionId", "name", "mode", "enabled", "changed", "needsUpdate")
    login = {}
    for path in (Path.home() / "Library/LaunchAgents").glob("*.plist"):
        value = plistlib.loads(path.read_bytes())
        if "mcplink" in path.name.lower() or any("mcplink" in str(v).lower() for v in value.get("ProgramArguments", [])):
            login[str(path)] = sha(path)
    result = {
        "source_files": {s["path"]: sha(Path(s["path"])) for s in status["sources"]},
        "bindings": sorted([{"sourceId": b.get("sourceId"), **{f: b.get(f) for f in fields}} for b in status["bindings"]], key=lambda b: json.dumps(b, sort_keys=True)),
        "connections": sorted([{"id": c["id"], "transport": c["transport"]} for c in status["connections"] if not c.get("detected")], key=lambda c: c["id"]),
        "gateways": sorted([{"id": g["id"], "members": g["members"], "editVersion": g["editVersion"]} for g in gateways], key=lambda g: g["id"]),
        "paused": status["gateway"]["paused"], "login_items": login,
    }
    binding = data / "keychain-helper/active.json"
    if binding.exists():
        helper = binding.with_name("mcplink-keychain")
        value = json.loads(binding.read_text())
        assert value["version"] == 1 and sha(helper) == value["sha256"], "Fixed credential helper or binding changed"
        result["keychain_helper"] = {"binding_sha256": sha(binding), "helper_sha256": sha(helper)}
    return result


def require_quiet_processes(app, cli, data):
    desktop = str(app / "Contents/MacOS/mcplink-desktop")
    output = subprocess.check_output(["ps", "-ww", "-axo", "pid=,comm="], text=True)
    desktop_pids = [int(p[0]) for line in output.splitlines()
                    if len(p := line.strip().split(None, 1)) == 2 and p[1] == desktop]
    assert desktop_pids, "Cold-start the installed App with --no-auth-ui before verification"
    runtime_file = data / "runtime.json"
    assert runtime_file.is_file(), "Start the quiet App/backend before verification"
    runtime = json.loads(runtime_file.read_text())
    daemon = runtime["pid"]
    for pid in [*desktop_pids, daemon]:
        argv = shlex.split(subprocess.check_output(["ps", "-ww", "-p", str(pid), "-o", "args="], text=True))
        expected = [desktop] if pid in desktop_pids else [str(cli), str(app / "Contents/MacOS/mcplink")]
        assert argv and argv[0] in expected, "Unexpected App/backend executable"
        options = argv[1:] if pid in desktop_pids else argv[1:argv.index("daemon")]
        assert "--no-auth-ui" in options, "App AND existing backend must be cold-started with --no-auth-ui; a quiet client cannot silence an interactive backend"


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--app", type=Path, default=Path("/Applications/mcplink.app"))
    parser.add_argument("--cli", type=Path, default=Path.home() / ".cargo/bin/mcplink")
    parser.add_argument("--data-dir", type=Path, default=Path(os.environ.get("MCPLINK_DATA_DIR", Path.home() / "Library/Application Support/io.mcplink.mcplink")))
    parser.add_argument("--snapshot-only", action="store_true")
    parser.add_argument("--baseline", type=Path)
    parser.add_argument("--observe-seconds", type=int, default=35)
    args = parser.parse_args()
    assert sys.platform == "darwin", "This deployment check is macOS-specific"
    os.umask(0o077); args.record.mkdir(parents=True, exist_ok=False)
    report = {"conclusion": "FAIL", "checked_at": datetime.now(timezone.utc).isoformat(), "credentials_exported": False}
    try:
        schema = json.loads(subprocess.check_output([str(args.cli), "schema"], text=True))["ok"]
        no_ui = any(a.get("long") == "no-auth-ui" for a in schema["command"]["arguments"])
        if not args.snapshot_only:
            assert no_ui, "CLI lacks the noninteractive Keychain check"
            assert args.observe_seconds >= 30, "Observe at least three desktop polling intervals"
            require_quiet_processes(args.app, args.cli, args.data_dir)
        before = snapshot(args.cli, args.data_dir, no_ui)
        (args.record / "snapshot.json").write_text(json.dumps(before, ensure_ascii=False, indent=2) + "\n")
        if args.snapshot_only:
            report.update(conclusion="PASS", snapshot_only=True, keychain_ui_disabled=no_ui)
            return
        if args.baseline:
            baseline = json.loads(args.baseline.read_text())
            assert before == baseline, "Existing configurations, bindings, gateway members or login preferences changed"
        helper = args.record / "macos-identity"
        subprocess.run(["/usr/bin/swiftc", str(ROOT / "scripts/macos_identity.swift"), "-o", str(helper)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        app_cli = args.app / "Contents/MacOS/mcplink"
        desktop = args.app / "Contents/MacOS/mcplink-desktop"
        assert sha(args.cli) == sha(app_cli), "Installed standalone CLI and App CLI differ"
        identity_paths = [args.cli, app_cli, desktop]
        if "keychain_helper" in before:
            identity_paths.append(args.data_dir / "keychain-helper/mcplink-keychain")
        audit = json.loads(subprocess.check_output([str(helper), *map(str, identity_paths)], text=True))
        signatures = {s["path"]: s for s in audit["signatures"]}
        for path in (args.cli, app_cli):
            assert signatures[str(path.resolve())]["identifier"] == "io.mcplink.cli", "Unstable CLI signing identifier"
        if "keychain_helper" in before:
            assert signatures[str(identity_paths[-1].resolve())]["identifier"] == "io.mcplink.keychain", "Invalid fixed helper identifier"
        assert all(s["valid"] for s in signatures.values()), "Invalid installed signature"
        assert all(p["valid"] for p in audit["processes"]), "A running Bridge/App/daemon still uses replaced code; reload the owning Agent MCP connection"
        assert any(p["path"] == str(desktop) for p in audit["processes"]), "Installed desktop is not running"
        certs = {s.get("certificate_sha1") for s in signatures.values()}
        assert len(certs) == 1 and None not in certs, "A stable common signing certificate is required for this real-device check"
        cli_req = signatures[str(args.cli.resolve())]["requirement"]
        assert cli_req == signatures[str(app_cli.resolve())]["requirement"], "CLI paths have different signing requirements"
        (args.record / "identity-audit.json").write_text(json.dumps(audit, ensure_ascii=False, indent=2) + "\n")
        checks = 0; deadline = time.monotonic() + args.observe_seconds
        while True:
            require_quiet_processes(args.app, args.cli, args.data_dir)
            assert snapshot(args.cli, args.data_dir, True) == before, "Deployment observation changed existing configuration"
            checks += 1
            (args.record / "observation.json").write_text(json.dumps({"noninteractive_checks": checks, "complete": time.monotonic() >= deadline}) + "\n")
            if time.monotonic() >= deadline:
                break
            time.sleep(min(5, max(0, deadline - time.monotonic())))
        runtime = json.loads((args.data_dir / "runtime.json").read_text())
        http = urllib.request.build_opener(urllib.request.ProxyHandler({}))
        with http.open(f'http://127.0.0.1:{runtime["port"]}/health', timeout=3) as response:
            health = json.load(response)
        assert health["application"] == "mcplink" and health["instance"] == runtime["instance"] and health["managementApi"] == 23
        executable = subprocess.check_output(["ps", "-o", "comm=", "-p", str(runtime["pid"])], text=True).strip()
        assert executable in (str(args.cli), str(app_cli)) and sha(Path(executable)) == sha(args.cli)
        report.update(conclusion="PASS", keychain_ui_disabled=True, noninteractive_checks=checks,
                      keychain_ui_disabled_in_app_and_backend=True,
                      observation_seconds=args.observe_seconds, all_running_code_valid=True,
                      same_signing_certificate=True, stable_cli_identifier=True, configurations_preserved=True,
                      managementApi=health["managementApi"], raw_business_response_saved=False)
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (args.record / "report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps(report, ensure_ascii=False))


if __name__ == "__main__":
    main()
