"""Extract release archives with native tools and verify signatures/CLI/licenses."""
import argparse
import hashlib
import json
from pathlib import Path
import subprocess
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[2]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--zip", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--require-notarization", action="store_true")
    args = parser.parse_args()
    args.record.mkdir(parents=True, exist_ok=False)
    report = {"scope": "Native archive extraction and offline CLI discovery; no user config or Vault access"}
    try:
        with zipfile.ZipFile(args.zip) as archive:
            assert archive.testzip() is None
            assert all(not item.extra and not item.comment for item in archive.infolist()), "ZIP contains extended metadata"
        app_dir, cli_dir = args.record / "app", args.record / "cli"
        app_dir.mkdir(); cli_dir.mkdir()
        subprocess.run(["ditto", "-x", "-k", str(args.zip.resolve()), str(app_dir.resolve())], check=True)
        subprocess.run(["tar", "-xzf", str(args.cli.resolve()), "-C", str(cli_dir.resolve())], check=True)
        app = app_dir / "mcplink.app"
        subprocess.run(["codesign", "--verify", "--deep", "--strict", str(app)], check=True)
        if args.require_notarization:
            subprocess.run(["xcrun", "stapler", "validate", str(app)], check=True)
            subprocess.run(["spctl", "--assess", "--type", "execute", str(app)], check=True)
        for binary in (app / "Contents/MacOS/mcplink", cli_dir / "mcplink"):
            schema = json.loads(subprocess.check_output([str(binary.resolve()), "schema"], text=True))["ok"]
            assert schema["version"] == json.loads((ROOT / "package.json").read_text())["version"]
        with tarfile.open(args.cli) as archive:
            members = archive.getmembers()
            assert {m.name for m in members} == {"mcplink", "LICENSE", "THIRD_PARTY_NOTICES.txt"}
            assert all(m.isfile() and m.uid == m.gid == 0 and not m.uname and not m.gname and not m.pax_headers for m in members)
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.txt"):
            assert sha(cli_dir / name) == sha(ROOT / name), f"Missing/stale {name}"
        assert sha(cli_dir / "mcplink") == sha(app / "Contents/MacOS/mcplink")
        report.update(conclusion="PASS", app_signature_valid=True, app_and_standalone_cli_runnable=True,
                      standalone_licenses_present=True, local_archive_metadata_absent=True)
    except Exception as error:
        report.update(conclusion="FAIL", error=str(error))
    (args.record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(report["conclusion"], report.get("error", ""))
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
