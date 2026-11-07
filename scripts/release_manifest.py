"""Check release versions and record distributable hashes without copying local data."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def release_version():
    package = json.loads((ROOT / "package.json").read_text())
    lock = json.loads((ROOT / "package-lock.json").read_text())
    cargo = tomllib.loads((ROOT / "Cargo.toml").read_text())
    tauri = json.loads((ROOT / "src-tauri/tauri.conf.json").read_text())
    versions = {"package": package["version"], "package_lock": lock["version"],
                "package_lock_root": lock["packages"][""]["version"],
                "cargo": cargo["workspace"]["package"]["version"], "tauri": tauri["version"]}
    if len(set(versions.values())) != 1:
        raise ValueError(f"Release versions differ: {versions}")
    return package["version"]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--artifact", type=Path, action="append", default=[])
    parser.add_argument("--binary", type=Path, action="append", default=[])
    parser.add_argument("--require-clean", action="store_true")
    args = parser.parse_args()
    args.record.mkdir(parents=True, exist_ok=True)
    report = {"executed_at": datetime.now(timezone.utc).isoformat(), "platform": platform.platform()}
    try:
        report["version"] = release_version()
        status = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True)
        if args.require_clean and status.strip():
            raise ValueError("Release requires a clean source checkout")
        head = subprocess.run(["git", "rev-parse", "HEAD"], cwd=ROOT, capture_output=True, text=True)
        report.update(revision=head.stdout.strip() if head.returncode == 0 else None,
                      source_dirty=bool(status.strip()),
                      node_version=(ROOT / ".node-version").read_text().strip(),
                      rust_toolchain=tomllib.loads((ROOT / "rust-toolchain.toml").read_text())["toolchain"]["channel"])
        names = subprocess.check_output(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"], cwd=ROOT).decode().split("\0")
        report["source_files"] = {name: sha(ROOT / name) for name in sorted(set(names)) if name and (ROOT / name).is_file()}
        report["binaries"] = {}
        for binary in args.binary:
            data = binary.read_bytes()
            for prefix in (str(ROOT), str(Path.home())):
                for spelling in {prefix, prefix.replace("\\", "/")}:
                    for encoding in ("utf-8", "utf-16-le"):
                        if spelling.encode(encoding) in data:
                            raise ValueError(f"Build-machine path leaked into {binary.name}")
            report["binaries"][binary.name] = {"sha256": sha(binary), "build_machine_paths_absent": True}
        names = [p.name for p in args.artifact]
        if len(set(names)) != len(names):
            raise ValueError("Artifact basenames must be unique")
        report["artifacts"] = {p.name: {"sha256": sha(p), "bytes": p.stat().st_size} for p in args.artifact}
        checksums = "".join(f"{item['sha256']}  {name}\n" for name, item in report["artifacts"].items())
        (args.record / "SHA256SUMS").write_text(checksums)
        report["conclusion"] = "PASS"
    except Exception as error:
        report.update(conclusion="FAIL", error=str(error))
    (args.record / "release-manifest.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(report["conclusion"], report.get("version", ""))
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
