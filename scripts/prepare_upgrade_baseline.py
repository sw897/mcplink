"""Fetch a recent successful default-branch installer for isolated upgrade acceptance."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--repo", required=True)
    parser.add_argument("--artifact", required=True)
    parser.add_argument("--platform", choices=["macos", "windows"], required=True)
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    args.record.mkdir(parents=True, exist_ok=False)
    report = {"available": False, "scope": "last successful push on main with an available installer artifact"}
    process = subprocess.run(["gh", "run", "list", "--repo", args.repo, "--workflow", "ci.yml", "--branch", "main", "--event", "push", "--status", "success", "--limit", "10", "--json", "databaseId,headSha"], capture_output=True, text=True, timeout=45)
    if process.returncode:
        raise RuntimeError("Cannot list upgrade baselines; verify GitHub authentication and Actions read permission")
    if process.returncode == 0:
        for run in json.loads(process.stdout)[:3]:
            destination = args.record / str(run["databaseId"])
            downloaded = subprocess.run(["gh", "run", "download", str(run["databaseId"]), "--repo", args.repo, "--name", args.artifact, "--dir", str(destination)], capture_output=True, text=True, timeout=180)
            if downloaded.returncode:
                if not any(message in downloaded.stderr.lower() for message in ["404", "410", "not found", "no artifacts", "no valid artifacts", "not exist", "expired"]):
                    raise RuntimeError("Cannot download the upgrade baseline; verify Actions read permission and network access")
                continue
            files = list(destination.rglob("*.dmg" if args.platform == "macos" else "*setup.exe"))
            if len(files) != 1 or files[0].is_symlink():
                continue
            installer = files[0].resolve()
            report.update(available=True, source_run=run["databaseId"], source_commit=run["headSha"], installer=installer.name, sha256=hashlib.sha256(installer.read_bytes()).hexdigest())
            variable = "MCPLINK_PREVIOUS_DMG" if args.platform == "macos" else "MCPLINK_PREVIOUS_INSTALLER"
            if os.environ.get("GITHUB_ENV"):
                with Path(os.environ["GITHUB_ENV"]).open("a", encoding="utf-8") as output:
                    output.write(f"{variable}={installer}\n")
            break
    if not report["available"]:
        report["limits"] = ["No accessible baseline installer: only current install/reinstall can be checked"]
    (args.record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("Upgrade baseline ready" if report["available"] else "No previous installer available; recorded upgrade limitation")


if __name__ == "__main__":
    main()
