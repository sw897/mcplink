"""Fetch a verified earlier installer; prefer a different version for upgrade acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--artifact", required=True)
    parser.add_argument("--platform", choices=["macos", "windows"], required=True)
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    args.record.mkdir(parents=True, exist_ok=False)
    current_version = json.loads((ROOT / "package.json").read_text())["version"]
    report = {"available": False, "current_version": current_version, "scope": "successful pushes on main with matching source manifests and installer versions"}
    process = subprocess.run(["gh", "run", "list", "--repo", args.repo, "--workflow", "ci.yml", "--branch", "main", "--event", "push", "--status", "success", "--limit", "10", "--json", "databaseId,headSha"], capture_output=True, text=True, timeout=45)
    if process.returncode:
        raise RuntimeError("Cannot list upgrade baselines; verify GitHub authentication and Actions read permission")
    fallback, chosen = None, None
    for run in json.loads(process.stdout)[:3]:
        destination = args.record / str(run["databaseId"])
        # Older CI used acceptance-* and included reports. New CI exposes only
        # packages and build provenance; keep those old installers usable.
        artifact_names = [args.artifact.replace("acceptance-", "packages-")]
        legacy = args.artifact.replace("packages-", "acceptance-")
        if legacy not in artifact_names:
            artifact_names.append(legacy)
        for artifact_name in artifact_names:
            downloaded = subprocess.run(["gh", "run", "download", str(run["databaseId"]), "--repo", args.repo, "--name", artifact_name, "--dir", str(destination)], capture_output=True, text=True, timeout=180)
            if downloaded.returncode == 0:
                break
            if not any(message in downloaded.stderr.lower() for message in ["404", "410", "not found", "no artifacts", "no artifact matches", "no valid artifacts", "not exist", "expired"]):
                raise RuntimeError("Cannot download the upgrade baseline; verify Actions read permission and network access")
        if downloaded.returncode:
            if not any(message in downloaded.stderr.lower() for message in ["404", "410", "not found", "no artifacts", "no artifact matches", "no valid artifacts", "not exist", "expired"]):
                raise RuntimeError("Cannot download the upgrade baseline; verify Actions read permission and network access")
            continue
        try:
            candidates = [destination / "artifacts/build/source/release-manifest.json", destination / "artifacts/e2e/ci-manifest/release-manifest.json"]
            manifest = json.loads(next(path for path in candidates if path.is_file()).read_text())
        except (OSError, ValueError, StopIteration):
            continue
        version = manifest.get("version")
        if manifest.get("conclusion") != "PASS" or manifest.get("revision") != run["headSha"] or not isinstance(version, str):
            continue
        packages = {}
        for path in destination.rglob("*.dmg" if args.platform == "macos" else "*setup.exe"):
            if path.is_symlink() or not path.is_file() or not path.name.startswith(f"mcplink_{version}_"):
                continue
            # Artifact caches can contain older versions, and the preview release
            # can also contain an identical copy of the current installer.
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            packages.setdefault(digest, path.resolve())
        if len(packages) != 1:
            continue
        digest, installer = next(iter(packages.items()))
        candidate = (run, installer, digest, version)
        if fallback is None:
            fallback = candidate
        if version != current_version:
            chosen = candidate
            break
    chosen = chosen or fallback
    if chosen:
        run, installer, digest, version = chosen
        report.update(available=True, source_run=run["databaseId"], source_commit=run["headSha"], baseline_version=version, cross_version=version != current_version, installer=installer.name, sha256=digest)
        variable = "MCPLINK_PREVIOUS_DMG" if args.platform == "macos" else "MCPLINK_PREVIOUS_INSTALLER"
        if os.environ.get("GITHUB_ENV"):
            with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as output:
                output.write(f"{variable}={installer}\n")
        if not report["cross_version"]:
            report["limits"] = ["Only an earlier build of the same version is available; this is not cross-version migration acceptance"]
    else:
        report["limits"] = ["No accessible matching baseline installer: only current install/reinstall can be checked"]
    (args.record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("Cross-version upgrade baseline ready" if report.get("cross_version") else "Earlier-build baseline ready" if report["available"] else "No previous installer available; recorded upgrade limitation")


if __name__ == "__main__":
    main()
