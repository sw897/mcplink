"""Prepare or reuse a fixed credential helper, without installing or reading credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
from macos_keychain_binding import active_helper

ROOT = Path(__file__).resolve().parents[1]


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--data-dir", type=Path, default=Path(os.environ.get("MCPLINK_DATA_DIR", Path.home() / "Library/Application Support/io.mcplink.mcplink")))
    parser.add_argument("--new-helper-candidate", action="store_true", help="Explicitly prepare a separate candidate when helper source changes; never replaces an installed helper")
    args = parser.parse_args()
    assert sys.platform == "darwin", "macOS only"
    os.umask(0o077)
    args.record = args.record.resolve()
    args.record.mkdir(parents=True, exist_ok=False)
    config = json.loads((ROOT / ".local/macos-signing.json").read_text())
    identity = config["identity"]
    assert len(identity) == 40 and all(c in "0123456789ABCDEFabcdef" for c in identity)
    source_hash = sha(ROOT / "scripts/macos_keychain.swift")
    cache = ROOT / ".local/keychain-helper"
    active = active_helper(args.data_dir)
    binary = args.record / "mcplink-keychain"
    report = {"protocol": 1, "identifier": "io.mcplink.keychain", "certificate_sha1": identity,
              "source_sha256": source_hash, "credentials_read": False, "installed_or_enabled": False}
    if active and not args.new_helper_candidate:
        shutil.copy2(active["path"], binary)
        report["mode"] = "reused_installed_bytes"
        report["current_source_sha256"] = source_hash
        report["source_sha256"] = None
        if (cache / "manifest.json").exists():
            cached = json.loads((cache / "manifest.json").read_text())
            if cached.get("sha256") == active["sha256"]:
                report["source_sha256"] = cached.get("source_sha256")
    elif (cache / "manifest.json").exists() and not args.new_helper_candidate:
        previous = json.loads((cache / "manifest.json").read_text())
        assert previous["source_sha256"] == source_hash and previous["certificate_sha1"] == identity, "Helper source/signer changed; review a separate --new-helper-candidate"
        assert sha(cache / "mcplink-keychain") == previous["sha256"], "Cached helper changed"
        shutil.copy2(cache / "mcplink-keychain", binary)
        report["mode"] = "reused_prepared_bytes"
    else:
        subprocess.run(["/usr/bin/swiftc", "-O", str(ROOT / "scripts/macos_keychain.swift"), "-o", str(binary)],
                       stdout=subprocess.DEVNULL, check=True)
        subprocess.run(["/usr/bin/codesign", "--force", "--sign", identity, "--identifier", "io.mcplink.keychain",
                        "--options", "runtime", "--timestamp=none", str(binary)], check=True)
        report["mode"] = "prepared_separate_candidate"
    subprocess.run(["/usr/bin/codesign", "--verify", "--strict", "-R",
                    '=identifier "io.mcplink.keychain" and certificate leaf = H"' + identity + '"', str(binary)], check=True)
    binary.chmod(0o700)
    report["sha256"] = sha(binary)
    (args.record / "manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    if not active and not args.new_helper_candidate and not cache.exists():
        cache.mkdir(parents=True)
        shutil.copy2(binary, cache / "mcplink-keychain")
        (cache / "manifest.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
