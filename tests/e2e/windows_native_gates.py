"""Run independent Windows release gates with isolated, synthetic inputs."""
import argparse
import hashlib
import json
import os
from pathlib import Path
from shutil import copyfile
import subprocess
import sys
import traceback
import zipfile

import run as e2e
from windows_installer import management_api, stop_daemon
from windows_native_storage import NativeStorage, seed_accounts, verify_ntfs


BASELINE_REVISION = "71fcdaad393108c42e70d7edc6b3c22c0926fb2e"
BASELINE_INSTALLER = ("mcplink_0.3.2_x64-setup.exe", "98bd86ee491181f9becf8efbe4daf94c8a24c13471dcadea32f006aff058eaea")
BASELINE_CLI = ("mcplink_cli_0.3.2_x86_64-pc-windows-msvc.zip", "a962f277abb3290a36b9b340e24241f86042d08a9a81dc77f722ca799ec794d9")
GATES = {"acl": "NTFS ACL", "credentials": "Credential Manager migration", "nsis": "NSIS overlay"}
VERSION = json.loads((e2e.ROOT / "package.json").read_text(encoding="utf-8"))["version"]
INSTALLER = f"mcplink_{VERSION}_x64-setup.exe"
RELEASE = e2e.ROOT / "target/release"


class Blocked(Exception):
    pass


def sha256(path):
    with path.open("rb") as file:
        return hashlib.file_digest(file, "sha256").hexdigest()


def read_json(path):
    return json.loads(path.read_text(encoding="utf-8"))


def require_file(path, reason):
    if not path.is_file():
        raise Blocked(reason)
    return path


def baseline_asset(record, asset):
    name, expected = asset
    root = record / "baseline/artifacts/build"
    artifact = require_file(root / "windows-release" / name, f"Baseline {name} is missing")
    manifest = read_json(require_file(root / "source/release-manifest.json", "Baseline manifest is missing"))
    if manifest["revision"] != BASELINE_REVISION or manifest["conclusion"] != "PASS":
        raise Blocked("Baseline source revision or verification is invalid")
    if sha256(artifact) != expected:
        raise Blocked(f"Baseline {name} SHA-256 differs from the pinned artifact")
    return artifact


def record_candidate(record):
    candidate = record / "candidate"
    candidate.mkdir()
    installer = candidate / INSTALLER
    copyfile(RELEASE / "bundle/nsis" / INSTALLER, installer)
    subprocess.run(
        [sys.executable, str(e2e.ROOT / "scripts/release_manifest.py"), "--require-clean",
         "--without-fixture-vault", "--record", str(candidate),
         "--artifact", str(installer), "--binary", str(RELEASE / "mcplink.exe"),
         "--binary", str(RELEASE / "mcplink-desktop.exe")],
        check=True,
    )


def nsis_inputs(record):
    # Only the fresh build stages an installer outside the Cargo cache.
    installer = require_file(record / "candidate" / INSTALLER, "Candidate NSIS artifact is missing")
    manifest = read_json(require_file(record / "candidate/release-manifest.json", "Candidate NSIS provenance is missing"))
    if (manifest["conclusion"] != "PASS" or manifest["revision"] != os.environ["GITHUB_SHA"]
            or manifest.get("fixture_vault_supported") is not False):
        raise Blocked("Candidate NSIS provenance does not match this commit")
    cli = require_file(RELEASE / "mcplink.exe", "Candidate release CLI is missing")
    if (manifest["artifacts"][INSTALLER]["sha256"] != sha256(installer)
            or manifest["binaries"][cli.name]["sha256"] != sha256(cli)):
        raise Blocked("Candidate NSIS or release CLI differs from its build manifest")
    return baseline_asset(record, BASELINE_INSTALLER), installer, cli


def harness_at(record):
    record.mkdir()
    return e2e.Harness(record)


def check_acl(record, fault):
    e2e.BINARY = require_file(record / "fixture/mcplink.exe", "Candidate fixture CLI is unavailable")
    h = harness_at(record / "acl-fixture")
    try:
        verify_ntfs(h, inject_failure=fault == "acl-failure")
    finally:
        h.close(False)


def check_credentials(record):
    current = require_file(record / "default/mcplink.exe", "Candidate default CLI is unavailable")
    archive = baseline_asset(record, BASELINE_CLI)
    previous = record / "previous-cli/mcplink.exe"
    previous.parent.mkdir()
    with zipfile.ZipFile(archive) as bundle:
        previous.write_bytes(bundle.read("mcplink.exe"))
    h = harness_at(record / "credentials-fixture")
    native = None
    try:
        native = NativeStorage(h)
        e2e.BINARY = previous
        token, oauth = seed_accounts(h)
        assert management_api(h) == 23
        native.prepare(oauth)
        stop_daemon(h, previous)

        e2e.BINARY = current
        for connection in (token, oauth):
            assert h.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
        assert management_api(h) == 24
        native.verify_migration()
        stop_daemon(h, current)
    finally:
        try:
            h.close(False)
        finally:
            if native:
                native.close()
    print("PASS native credentials retained and synthetic test entries removed", flush=True)


def check_installer(record, integrated=False):
    previous, installer, cli = nsis_inputs(record)
    destination = record / ("installed-mcp" if integrated else "nsis-fixture")
    subprocess.run(
        [sys.executable, "-u", str(e2e.ROOT / "tests/e2e/windows_installer.py"),
         "--native-migration" if integrated else "--native-overlay",
         "--installer", str(installer), "--previous-installer", str(previous), "--record", str(destination)],
        env=os.environ | {"MCPLINK_E2E_BINARY": str(cli)}, check=True,
    )
    report = read_json(destination / "report.json")
    assert report["conclusion"] == "PASS"
    assert (report["previous_management_api"], report["management_api"]) == (23, 24)
    print(f"PASS NSIS {report['installer_sha256']} upgraded management API 23 to 24", flush=True)
    if integrated:
        assert report["synthetic_credentials_removed"]


def run_gate(args):
    try:
        assert os.name == "nt", "Windows native acceptance requires a Windows runner"
        if args.step == "acl":
            check_acl(args.record, args.fault)
        elif args.step == "credentials":
            check_credentials(args.record)
        else:
            check_installer(args.record)
        result = {"status": "PASS", "detail": "Native checks completed"}
    except Blocked as error:
        result = {"status": "BLOCKED", "detail": str(error)}
    except Exception as error:
        traceback.print_exc()
        detail = "Injected NTFS ACL mismatch" if str(error) == "Injected NTFS ACL mismatch" else "Native check failed; see step log"
        result = {"status": "FAIL", "detail": detail}
    file = args.record / f"{args.step}.json"
    temporary = file.with_suffix(".tmp")
    temporary.write_text(json.dumps(result) + "\n", encoding="utf-8")
    temporary.replace(file)
    print(f"WINDOWS_NATIVE_RESULT {args.step} {result['status']} {result['detail']}", flush=True)
    return result["status"] == "PASS"


def summary(args):
    commit = os.environ.get("GITHUB_SHA", "local")
    rows = ["### Windows native release gates", "", f"Commit `{commit}` · Mode `{args.fault}`", "",
            "| Gate | Result | Detail |", "| --- | --- | --- |"]
    states = []
    for gate, label in GATES.items():
        file = args.record / f"{gate}.json"
        result = read_json(file) if file.is_file() else {"status": "FAIL", "detail": "No result was recorded"}
        rows.append(f"| {label} | {result['status']} | {result['detail']} |")
        states.append(f"{gate}={result['status']}")
    if path := os.environ.get("GITHUB_STEP_SUMMARY"):
        with Path(path).open("a", encoding="utf-8") as output:
            output.write("\n".join(rows) + "\n")
    print("\n".join(rows), flush=True)
    print(f"WINDOWS_NATIVE_SUMMARY {commit} {args.fault} {' '.join(states)}", flush=True)
    return args.fault == "none" and all(state.endswith("=PASS") for state in states)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("step", choices=[*GATES, "candidate", "installed-mcp", "summary"])
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--fault", choices=["none", "acl-failure", "missing-nsis"], default="none")
    args = parser.parse_args()
    args.record = args.record.resolve()
    args.record.mkdir(parents=True, exist_ok=True)
    if args.step == "candidate":
        record_candidate(args.record)
    elif args.step == "installed-mcp":
        check_installer(args.record, integrated=True)
    elif args.step == "summary":
        raise SystemExit(not summary(args))
    else:
        raise SystemExit(not run_gate(args))


if __name__ == "__main__":
    main()
