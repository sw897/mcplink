"""Add verified artifacts to a draft without replacing files or publishing it."""
import argparse
import hashlib
import json
from pathlib import Path
import re
import subprocess
import shutil
import tempfile


def prepare(directory, platform, tag, staging):
    manifest = json.loads((directory / "release-manifest.json").read_text())
    version = manifest["version"]
    assert tag == "v" + version, "Tag/version mismatch"
    assert manifest["conclusion"] == "PASS" and manifest["source_dirty"] is False, "A clean verified source build is required"
    assert re.fullmatch(r"[0-9a-f]{40}", manifest["revision"]), "Missing source revision"
    for name, checksum in manifest["source_files"].items():
        path = Path(name)
        assert not path.is_absolute() and ".." not in path.parts and "\\" not in name, "Invalid source path"
        assert not any(part.startswith(".") for part in path.parts if part not in {".github", ".node-version", ".npmrc", ".gitignore", ".gitattributes"}), "Private source path"
        assert path.name not in {"AGENTS.md", "CLAUDE.md"} and path.parts[0] not in {"artifacts", "target", "node_modules", "private_keys"} and "project-notes" not in path.parts, "Private source path"
        assert re.fullmatch(r"[0-9a-f]{64}", checksum), "Invalid source checksum"
    verification = json.loads((directory / "verification.json").read_text())
    assert verification["version"] == version
    if platform == "macos":
        assert all(verification[key] == "PASS" for key in ("package", "installed", "agent_cli", "lifecycle")), "Incomplete macOS checks"
        signing = {key: verification[key] for key in ("developer_id_signed", "notarized")}
        patterns = [f"mcplink_{version}_aarch64.dmg", f"mcplink_{version}_aarch64.app.zip", f"mcplink_cli_{version}_aarch64-apple-darwin.tar.gz"]
    else:
        suites = ("agent_cli", "aggregate_gateway", "entry_policy", "catalog_refresh", "cli_workflow", "daemon_lifecycle")
        assert verification["installed"]["conclusion"] == "PASS" and all(verification["suites"].get(key) == "PASS" for key in suites), "Incomplete Windows checks"
        signing = {"publisherSignatureVerified": verification["publisherSignatureVerified"]}
        patterns = [f"mcplink_{version}_x64-setup.exe", f"mcplink_cli_{version}_x86_64-pc-windows-msvc.zip"]
    expected = set(patterns)
    assert set(manifest["artifacts"]) == expected, "Unexpected distribution file set"
    checksums = {}
    for line in (directory / "SHA256SUMS").read_text().splitlines():
        checksum, name = line.split("  ", 1)
        assert re.fullmatch(r"[0-9a-f]{64}", checksum) and name in expected and name not in checksums, "Invalid checksum entry"
        checksums[name] = checksum
    assert set(checksums) == expected, "Incomplete checksums"
    for name in sorted(expected):
        path = directory / name
        assert path.is_file() and not path.is_symlink(), "Invalid artifact file"
        assert hashlib.sha256(path.read_bytes()).hexdigest() == checksums[name] == manifest["artifacts"][name]["sha256"], "Artifact checksum mismatch"
        assert path.stat().st_size == manifest["artifacts"][name]["bytes"], "Artifact size mismatch"
        shutil.copyfile(path, staging / name)
    # Only public build provenance and signing flags leave the candidate folder.
    # Raw verification, logs, screenshots and other directory contents stay local.
    public = {key: manifest[key] for key in ("version", "revision", "source_dirty", "node_version", "rust_toolchain", "source_files", "artifacts")}
    public["artifacts"] = {name: {key: manifest["artifacts"][name][key] for key in ("sha256", "bytes")} for name in sorted(expected)}
    assert all(isinstance(value, bool) for value in signing.values()), "Invalid signing status"
    public.update(platform=platform, signing=signing)
    (staging / f"{platform}-release-manifest.json").write_text(json.dumps(public, indent=2) + "\n")
    shutil.copyfile(directory / "SHA256SUMS", staging / f"{platform}-SHA256SUMS")
    return sorted(path for path in staging.iterdir() if path.is_file())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--platform", choices=["macos", "windows"], required=True)
    parser.add_argument("--output", type=Path, help="New directory for the curated public files")
    parser.add_argument("--prepare-only", action="store_true", help="Validate and curate files without GitHub writes")
    args = parser.parse_args()
    assert not args.prepare_only or args.output, "--prepare-only requires --output"
    temporary = None
    if args.output:
        staging = args.output
        staging.mkdir(parents=True, exist_ok=False)
    else:
        temporary = tempfile.TemporaryDirectory(prefix="mcplink-release-metadata-")
        staging = Path(temporary.name)
    artifacts = prepare(args.directory, args.platform, args.tag, staging)
    if args.prepare_only:
        print(json.dumps({"files": [path.name for path in artifacts], "revision": json.loads((args.directory / "release-manifest.json").read_text())["revision"]}))
        return
    result = subprocess.run(["gh", "release", "view", args.tag, "--json", "isDraft,tagName"], capture_output=True, text=True)
    if result.returncode == 0:
        release = json.loads(result.stdout)
        assert release["isDraft"] and release["tagName"] == args.tag, "Only an existing matching draft may receive artifacts"
        subprocess.run(["gh", "release", "upload", args.tag, *map(str, artifacts)], check=True)
    else:
        subprocess.run(["gh", "release", "create", args.tag, *map(str, artifacts), "--verify-tag", "--draft", "--title", f"mcplink {args.tag}", "--notes-file", "docs/release-notes.md"], check=True)


if __name__ == "__main__":
    main()
