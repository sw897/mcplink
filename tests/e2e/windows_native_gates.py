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


# Published v0.3.2 Windows release assets and the revision that built them.
BASELINE_VERSION = "0.3.2"
BASELINE_REVISION = "61e92213701848d565b9efa6990f7588eb17579a"
BASELINE_MANAGEMENT_API = 22
MANAGEMENT_API = 24
BASELINE_MANIFEST = (
    "windows-release-manifest.json",
    "a09ae1f797ef21971cfbe89412cebc8d0e498f558b672c95f35d200744f9e695",
)
BASELINE_INSTALLER = (
    f"mcplink_{BASELINE_VERSION}_x64-setup.exe",
    "e516eaa369731f324ffd3bb4904651335b31e982b0669054ef23e68d460617c3",
)
BASELINE_CLI = (
    f"mcplink_cli_{BASELINE_VERSION}_x86_64-pc-windows-msvc.zip",
    "6dbe2ed0d2744df422b09398497d6c95d67bd92c64dc873d900f936662d6a9f0",
)
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


def download_baseline(record):
    destination = record / "baseline"
    destination.mkdir()
    command = ["gh", "release", "download", f"v{BASELINE_VERSION}", "--repo", "sw897/mcplink",
               "--dir", str(destination)]
    for name, _ in (BASELINE_MANIFEST, BASELINE_CLI, BASELINE_INSTALLER):
        command.extend(["--pattern", name])
    subprocess.run(command, check=True, timeout=180)


def baseline_file(record, asset):
    name, expected = asset
    path = require_file(record / "baseline" / name, f"Baseline {name} is missing")
    if sha256(path) != expected:
        raise Blocked(f"Baseline {name} SHA-256 differs from the pinned release")
    return path


def baseline_asset(record, asset):
    name, expected = asset
    path = baseline_file(record, asset)
    manifest = read_json(baseline_file(record, BASELINE_MANIFEST))
    if (manifest.get("version") != BASELINE_VERSION or manifest.get("revision") != BASELINE_REVISION
            or manifest.get("platform") != "windows" or manifest.get("source_dirty") is not False):
        raise Blocked("Baseline release manifest does not match the pinned Windows source")
    published = manifest["artifacts"].get(name, {})
    if published.get("sha256") != expected or published.get("bytes") != path.stat().st_size:
        raise Blocked(f"Baseline {name} differs from its published manifest")
    print(f"PASS public baseline v{BASELINE_VERSION} {name} SHA-256 {expected}", flush=True)
    return path


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
        assert management_api(h) == BASELINE_MANAGEMENT_API
        native.prepare(oauth)
        stop_daemon(h, previous)

        e2e.BINARY = current
        for connection in (token, oauth):
            assert h.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
        assert management_api(h) == MANAGEMENT_API
        native.verify_migration()
        stop_daemon(h, current)
    finally:
        try:
            h.close(False)
        finally:
            if native:
                native.close()
    print(f"PASS native credentials migrated from management API {BASELINE_MANAGEMENT_API} to {MANAGEMENT_API}"
          " and synthetic test entries were removed", flush=True)


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
    assert (report["previous_management_api"], report["management_api"]) == (BASELINE_MANAGEMENT_API, MANAGEMENT_API)
    print(f"PASS NSIS {report['installer_sha256']} upgraded management API {BASELINE_MANAGEMENT_API} to {MANAGEMENT_API}", flush=True)
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
    parser.add_argument("step", choices=[*GATES, "download-baseline", "candidate", "installed-mcp", "summary"])
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--fault", choices=["none", "acl-failure", "missing-nsis"], default="none")
    args = parser.parse_args()
    args.record = args.record.resolve()
    args.record.mkdir(parents=True, exist_ok=True)
    if args.step == "download-baseline":
        download_baseline(args.record)
    elif args.step == "candidate":
        record_candidate(args.record)
    elif args.step == "installed-mcp":
        check_installer(args.record, integrated=True)
    elif args.step == "summary":
        raise SystemExit(not summary(args))
    else:
        raise SystemExit(not run_gate(args))


if __name__ == "__main__":
    main()
