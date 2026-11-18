"""Install a locally signed candidate, preserving data and recording reconnect work."""
import argparse
import json
import os
from pathlib import Path
import shlex
import shutil
import signal
import subprocess
import sys
import time

from verify_macos_deployment import ROOT, sha, snapshot


def process_rows():
    output = subprocess.check_output(["ps", "-axo", "pid=,comm="], text=True)
    return [(int(p[0]), p[1]) for line in output.splitlines() if len(p := line.strip().split(None, 1)) == 2]


def wait_exit(pid):
    deadline = time.monotonic() + 35
    while True:
        state = subprocess.run(["ps", "-p", str(pid), "-o", "stat="], capture_output=True, text=True).stdout.strip()
        if not state or state.startswith("Z"):
            return
        if time.monotonic() >= deadline:
            raise RuntimeError(f"Process {pid} did not exit gracefully; no forced kill was issued")
        time.sleep(.1)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--build", action="store_true")
    parser.add_argument("--allow-auth-ui", action="store_true",
                        help="Explicitly allow system Keychain dialogs during first-time migration; default launch is noninteractive")
    parser.add_argument("--app", type=Path, default=ROOT / "target/release/bundle/macos/mcplink.app")
    parser.add_argument("--target", type=Path, default=Path("/Applications/mcplink.app"))
    parser.add_argument("--cli", type=Path, default=Path.home() / ".cargo/bin/mcplink")
    parser.add_argument("--data-dir", type=Path, default=Path(os.environ.get("MCPLINK_DATA_DIR", Path.home() / "Library/Application Support/io.mcplink.mcplink")))
    parser.add_argument("--stop-bridge", type=int, action="append", default=[])
    args = parser.parse_args()
    assert sys.platform == "darwin", "macOS only"
    args.record = args.record.resolve()
    args.app = args.app.resolve()
    assert args.target.is_absolute() and not args.target.is_symlink()
    assert args.cli.is_absolute() and args.cli.is_file() and not args.cli.is_symlink()
    assert args.data_dir.is_absolute(), "Keep the exact existing absolute data-directory namespace"
    os.umask(0o077); args.record.mkdir(parents=True, exist_ok=False)
    result = {"state": "preparing", "credentials_or_database_modified": False}
    try:
        if args.build:
            subprocess.run(["npm", "run", "bundle", "--", "--local-signing", "--bundles", "app"], cwd=ROOT, check=True)
        helper = args.record / "macos-identity"
        subprocess.run(["swiftc", str(ROOT / "scripts/macos_identity.swift"), "-o", str(helper)],
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=True)
        built_cli = args.app / "Contents/MacOS/mcplink"
        built_desktop = args.app / "Contents/MacOS/mcplink-desktop"
        identity = json.loads(subprocess.check_output([str(helper), str(built_cli), str(built_desktop)], text=True))
        assert all(s["valid"] and s.get("certificate_sha1") for s in identity["signatures"]), "Candidate needs a valid certificate signature"
        assert {s["identifier"] for s in identity["signatures"]} == {"io.mcplink.cli", "io.mcplink.desktop"}
        assert len({s["certificate_sha1"] for s in identity["signatures"]}) == 1
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(args.app)], capture_output=True, check=True)
        schema = json.loads(subprocess.check_output([str(args.cli), "schema"], text=True))["ok"]
        no_ui = any(a.get("long") == "no-auth-ui" for a in schema["command"]["arguments"])
        assert no_ui or args.allow_auth_ui, "Old CLI lacks --no-auth-ui; review first-time authorization and explicitly select --allow-auth-ui"
        before = snapshot(args.cli, args.data_dir, no_ui)
        if "keychain_helper" in before:
            fixed_helper = args.data_dir / "keychain-helper/mcplink-keychain"
            helper_audit = json.loads(subprocess.check_output([str(helper), str(fixed_helper)], text=True))["signatures"][0]
            assert helper_audit["valid"] and helper_audit["identifier"] == "io.mcplink.keychain"
            assert helper_audit["certificate_sha1"] == identity["signatures"][0]["certificate_sha1"], "Candidate signer differs from the fixed helper; review migration before replacing working software"
        (args.record / "before.json").write_text(json.dumps(before, ensure_ascii=False, indent=2) + "\n")
        old_cli_sha = sha(args.cli)
        # Only explicitly selected, currently verified native Bridge processes.
        rows = dict(process_rows())
        for pid in set(args.stop_bridge):
            assert rows.get(pid) == str(args.cli), "Selected PID is not this installed CLI"
            command = shlex.split(subprocess.check_output(["ps", "-p", str(pid), "-o", "command="], text=True))
            assert len(command) > 1 and command[0] == str(args.cli) and command[1] == "bridge", "Selected PID is not a native Bridge"
        stage = args.target.with_name(f".mcplink-next-{os.getpid()}.app")
        previous = args.target.with_name(f".mcplink-previous-{os.getpid()}.app")
        assert not stage.exists() and not previous.exists()
        subprocess.run(["ditto", str(args.app), str(stage)], check=True)
        for pid in set(args.stop_bridge):
            os.kill(pid, signal.SIGTERM); wait_exit(pid)
        stop = [str(args.cli), "--data-dir", str(args.data_dir)]
        if no_ui and not args.allow_auth_ui:
            stop.append("--no-auth-ui")
        # An old desktop lacking permission to the running backend can still
        # quit cleanly once that backend has been stopped by the reviewed CLI.
        subprocess.run([*stop, "gateway", "stop"], capture_output=True, check=True, timeout=35)
        for pid, path in process_rows():
            if path == str(args.target / "Contents/MacOS/mcplink-desktop"):
                os.kill(pid, signal.SIGTERM); wait_exit(pid)
        subprocess.run([*stop, "gateway", "stop"], capture_output=True, check=True, timeout=35)
        assert sha(args.cli) == old_cli_sha, "Installed CLI changed concurrently"
        rollback = args.record / "rollback"; rollback.mkdir()
        shutil.copy2(args.cli, rollback / "mcplink")
        cli_stage = args.cli.with_name(f".mcplink-next-{os.getpid()}")
        assert not cli_stage.exists()
        shutil.copy2(stage / "Contents/MacOS/mcplink", cli_stage); cli_stage.chmod(0o755)
        os.rename(args.target, previous)
        moved_app = False
        replaced_cli = False
        try:
            os.rename(stage, args.target)
            moved_app = True
            os.replace(cli_stage, args.cli)
            replaced_cli = True
            assert sha(args.cli) == sha(args.target / "Contents/MacOS/mcplink")
            subprocess.run(["codesign", "--verify", "--deep", "--strict", str(args.target)], capture_output=True, check=True)
        except BaseException:
            if moved_app:
                os.rename(args.target, stage)
            os.rename(previous, args.target)
            if replaced_cli:
                shutil.copy2(rollback / "mcplink", cli_stage)
                os.replace(cli_stage, args.cli)
            result["state"] = "rolled_back_pending_agent_reload"
            raise
        result.update(state="installed_pending_authorization_and_agent_reload", previous_app=str(previous), rollback=str(rollback))
        subprocess.run(["ditto", str(previous), str(rollback / "mcplink.app")], check=True)
        assert sha(previous / "Contents/MacOS/mcplink-desktop") == sha(rollback / "mcplink.app/Contents/MacOS/mcplink-desktop")
        shutil.rmtree(previous)
        result.update(state="installed_pending_authorization_and_agent_reload", installed_app=str(args.target),
                      cli_sha256=sha(args.cli), app_desktop_sha256=sha(args.target / "Contents/MacOS/mcplink-desktop"),
                      stopped_bridge_pids=sorted(set(args.stop_bridge)), rollback=str(rollback),
                      source_commit=subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip())
        result["source_dirty"] = bool(subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True))
        result.pop("previous_app", None)
        result["keychain_ui_disabled_on_launch"] = not args.allow_auth_ui
        launch = ["open", "-a", str(args.target)]
        if not args.allow_auth_ui:
            launch.extend(["--args", "--no-auth-ui"])
        subprocess.run(launch, check=True)
    finally:
        (args.record / "installation.json").write_text(json.dumps(result, ensure_ascii=False, indent=2) + "\n")
        print(json.dumps(result, ensure_ascii=False))
    print("Reload the owning Agents' MCP connections and run verify_macos_deployment.py with before.json as the baseline. Default launch cannot show Keychain dialogs; missing grants require a separately reviewed interactive migration.")


if __name__ == "__main__":
    main()
