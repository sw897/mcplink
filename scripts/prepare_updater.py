"""Create version-bound, signed updater files from a verified release candidate."""
import argparse
import gzip
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import plistlib
import shutil
import stat
import subprocess
import tarfile
import tempfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]
PUBLIC_KEY = ROOT / "src-tauri/updater.pub"
MAX_PACKAGE_BYTES = 256 * 1024 * 1024


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def names(platform, version):
    if platform == "macos":
        return ("darwin-aarch64", f"mcplink_update_{version}_aarch64.app.tar.gz", "macos-updater.json")
    return ("windows-x86_64", f"mcplink_{version}_x64-setup.exe", "windows-updater.json")


def verifier():
    subprocess.run(["cargo", "build", "--locked", "-q", "-p", "mcplink-desktop", "--example", "updater_verify"], cwd=ROOT, check=True)
    return ROOT / "target/debug/examples" / ("updater_verify.exe" if os.name == "nt" else "updater_verify")


def verify_signature(program, key, payload, signature, version):
    result = subprocess.run([str(program), str(key), str(payload), str(signature), version], capture_output=True, text=True, encoding="utf-8")
    if result.returncode:
        raise ValueError("Updater signature, key or signed version did not verify")
    report = json.loads(result.stdout)
    assert report["verified"] and report["sha256"] == sha(payload), "Updater verification mismatch"
    return report


def app_tar(source, output, version, fixture=False):
    """Convert the already verified distribution ZIP; do not accept another app path."""
    with zipfile.ZipFile(source) as archive:
        entries = archive.infolist()
        assert entries and sum(item.file_size for item in entries) <= 512 * 1024 * 1024, "Oversize app ZIP"
        paths = set()
        for item in entries:
            path = PurePosixPath(item.filename)
            assert item.filename and not path.is_absolute() and ".." not in path.parts and "\\" not in item.filename, "Unsafe app ZIP path"
            assert path.parts[0] == "mcplink.app" and item.filename not in paths, "Unexpected app ZIP member"
            paths.add(item.filename)
            mode = item.external_attr >> 16
            assert item.is_dir() or stat.S_ISREG(mode), "Updater app entries must be directories or regular files"
        info = plistlib.loads(archive.read("mcplink.app/Contents/Info.plist"))
        assert info["CFBundleShortVersionString"] == version, "App/version mismatch"
        identifier = info["CFBundleIdentifier"]
        assert identifier == "io.mcplink.desktop" or (fixture and identifier.startswith("io.mcplink.acceptance.")), "Unexpected app identifier"
        for name in ("mcplink", "mcplink-desktop"):
            entry = archive.getinfo("mcplink.app/Contents/MacOS/" + name)
            assert (entry.external_attr >> 16) & 0o111, "App binary is not executable"
        with output.open("xb") as raw, gzip.GzipFile(fileobj=raw, mode="wb", filename="", mtime=0) as compressed, tarfile.open(fileobj=compressed, mode="w") as tar:
            for item in sorted(entries, key=lambda e: e.filename):
                mode = item.external_attr >> 16
                member = tarfile.TarInfo(item.filename.rstrip("/"))
                member.uid = member.gid = member.mtime = 0
                member.uname = member.gname = ""
                if item.is_dir():
                    member.type, member.mode = tarfile.DIRTYPE, 0o755
                    tar.addfile(member)
                else:
                    member.mode = 0o755 if mode & 0o111 else 0o644
                    member.size = item.file_size
                    with archive.open(item) as content:
                        tar.addfile(member, content)


def check_candidate(directory, manifest):
    assert manifest["conclusion"] == "PASS", "Candidate source checks failed"
    for name, item in manifest["artifacts"].items():
        path = directory / name
        assert Path(name).name == name and not path.is_symlink() and path.is_file(), "Invalid candidate artifact"
        assert sha(path) == item["sha256"] and path.stat().st_size == item["bytes"], "Candidate artifact changed"


def verify_update(directory, platform, version, program=None, key=PUBLIC_KEY):
    """Used again before upload; never trust an earlier verification boolean."""
    target, payload_name, manifest_name = names(platform, version)
    manifest = json.loads((directory / manifest_name).read_text())
    assert manifest["version"] == version and set(manifest["platforms"]) == {target}, "Updater platform/version mismatch"
    entry = manifest["platforms"][target]
    assert entry["url"] == f"https://github.com/sw897/mcplink/releases/download/v{version}/{payload_name}", "Unexpected updater URL"
    payload, signature = directory / payload_name, directory / (payload_name + ".sig")
    assert all(p.is_file() and not p.is_symlink() for p in (payload, signature)), "Invalid updater artifact"
    assert 0 < entry["size"] <= MAX_PACKAGE_BYTES and payload.stat().st_size == entry["size"] and sha(payload) == entry["sha256"], "Updater size/hash mismatch"
    assert signature.stat().st_size <= 16384 and signature.read_text().strip() == entry["signature"], "Updater signature file mismatch"
    verify_signature(program or verifier(), key, payload, signature, version)
    return {payload_name, payload_name + ".sig", manifest_name}


def prepare(directory, platform, key=PUBLIC_KEY, fixture=False):
    manifest = json.loads((directory / "release-manifest.json").read_text())
    verification = json.loads((directory / "verification.json").read_text())
    version = manifest["version"]
    assert verification["version"] == version, "Candidate verification version mismatch"
    check_candidate(directory, manifest)
    _, payload_name, manifest_name = names(platform, version)
    additions = {payload_name + ".sig", manifest_name}
    if platform == "macos":
        additions.add(payload_name)
    assert not any((directory / name).exists() for name in additions), "Updater files already exist; use a fresh candidate"
    assert os.environ.get("TAURI_SIGNING_PRIVATE_KEY") or os.environ.get("TAURI_SIGNING_PRIVATE_KEY_PATH"), "Updater signing key is required"
    program = verifier()
    with tempfile.TemporaryDirectory(prefix=".mcplink-updater-", dir=directory) as temporary:
        stage = Path(temporary)
        payload = stage / payload_name
        if platform == "macos":
            source_name = f"mcplink_{version}_aarch64.app.zip"
            assert source_name in manifest["artifacts"], "Verified app ZIP is required"
            app_tar(directory / source_name, payload, version, fixture)
        else:
            assert payload_name in manifest["artifacts"], "Verified NSIS installer is required"
            shutil.copyfile(directory / payload_name, payload)
        assert 0 < payload.stat().st_size <= MAX_PACKAGE_BYTES, "Update package exceeds download limit"
        signer = ROOT / "node_modules/@tauri-apps/cli/tauri.js"
        # Signer output can contain key material: capture it and report only success/failure.
        result = subprocess.run(["node", str(signer), "signer", "sign", "--app-version", version, payload.name], cwd=stage, capture_output=True, text=True)
        assert result.returncode == 0, "Updater signing failed; check signing key environment"
        signature = stage / (payload.name + ".sig")
        report = verify_signature(program, key, payload, signature, version)
        target = names(platform, version)[0]
        metadata = {"version": version, "notes": f"mcplink {version}", "platforms": {target: {
            "url": f"https://github.com/sw897/mcplink/releases/download/v{version}/{payload_name}",
            "signature": signature.read_text().strip(), "sha256": report["sha256"], "size": report["bytes"],
        }}}
        (stage / manifest_name).write_text(json.dumps(metadata, indent=2) + "\n")
        verify_update(stage, platform, version, program, key)
        for name in sorted(additions):
            shutil.copyfile(stage / name, directory / name)
    for name in additions:
        manifest["artifacts"][name] = {"sha256": sha(directory / name), "bytes": (directory / name).stat().st_size}
    verification["updaterSignatureVerified"] = True
    (directory / "release-manifest.json").write_text(json.dumps(manifest, indent=2) + "\n")
    (directory / "verification.json").write_text(json.dumps(verification, indent=2) + "\n")
    (directory / "SHA256SUMS").write_text("".join(f"{item['sha256']}  {name}\n" for name, item in sorted(manifest["artifacts"].items())))
    return sorted(additions)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--platform", choices=["macos", "windows"], required=True)
    # No fixture endpoint or key override is available to the public upload command.
    parser.add_argument("--fixture-home", type=Path, help="Marked isolated acceptance home only")
    parser.add_argument("--public-key", type=Path)
    args = parser.parse_args()
    key = PUBLIC_KEY
    if args.fixture_home:
        home = args.fixture_home.resolve()
        assert (home / ".mcplink-test-fixture").is_file(), "Missing fixture marker"
        assert args.directory.resolve().is_relative_to(home), "Candidate outside fixture home"
        assert args.public_key and args.public_key.resolve().is_relative_to(home), "Fixture public key outside home"
        assert args.public_key.read_text().strip() != PUBLIC_KEY.read_text().strip(), "Do not use the production key for fixtures"
        key = args.public_key
    else:
        assert not args.public_key, "Public key override requires an isolated fixture"
    files = prepare(args.directory.resolve(), args.platform, key, bool(args.fixture_home))
    print(json.dumps({"updaterSignatureVerified": True, "files": files}))


if __name__ == "__main__":
    main()
