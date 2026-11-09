"""Prepare Tauri's pinned Windows NSIS tools via the GitHub Release API."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import time
import zipfile


ROOT = Path(__file__).resolve().parents[1]
TAURI_CLI_VERSION = "2.12.4"
# Asset SHA-256 digests published by the official releases. The contents also
# match the SHA-1 pins and layout in the locked CLI's bundler implementation:
# https://github.com/tauri-apps/tauri/blob/tauri-cli-v2.12.4/crates/tauri-bundler/src/bundle/windows/nsis/mod.rs
ASSETS = (
    (
        "tauri-apps/binary-releases", "nsis-3.13", "nsis-3.13.zip",
        "ba63dffc4410ee89193e1cb5a41989991bd77c61068da17e3156d136b7b0b3d8",
    ),
    (
        "tauri-apps/nsis-tauri-utils", "nsis_tauri_utils-v0.5.3", "nsis_tauri_utils.dll",
        "5ba143b5db4a87d32d6e7802e033330aae56cbceabe0d1e3ba41948385ad4709",
    ),
)


def verified(path, digest):
    if not path.is_file():
        return False
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest() == digest


def download_asset(cache, repo, tag, name, digest):
    target = cache / name
    if verified(target, digest):
        print(f"Verified cached {name}", flush=True)
        return target
    with tempfile.TemporaryDirectory(prefix="download-", dir=cache) as temporary:
        incoming = Path(temporary) / name
        for attempt in range(1, 5):
            print(f"Downloading {name} via GitHub Release API ({attempt}/4)", flush=True)
            try:
                result = subprocess.run(
                    ["gh", "release", "download", tag, "--repo", repo,
                     "--pattern", name, "--output", str(incoming), "--clobber"],
                    timeout=30,
                    check=False,
                )
                succeeded = result.returncode == 0
            except subprocess.TimeoutExpired:
                succeeded = False
            if succeeded:
                if not verified(incoming, digest):
                    raise RuntimeError(f"SHA-256 mismatch for {name}; refusing to install")
                incoming.replace(target)
                return target
            if attempt < 4:
                time.sleep(attempt * 5)
        raise RuntimeError(f"Could not download {name} after 4 attempts")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--tools-dir", type=Path, help="Tauri cache root; defaults to LOCALAPPDATA/tauri on Windows")
    args = parser.parse_args()
    lock = json.loads((ROOT / "package-lock.json").read_text(encoding="utf-8"))
    if lock["packages"]["node_modules/@tauri-apps/cli"]["version"] != TAURI_CLI_VERSION:
        parser.error("Tauri CLI changed; review the pinned NSIS assets and cache layout first")
    tools = args.tools_dir
    if tools is None:
        if os.name != "nt" or not os.environ.get("LOCALAPPDATA"):
            parser.error("--tools-dir is required outside Windows")
        tools = Path(os.environ["LOCALAPPDATA"]) / "tauri"
    tools = tools.resolve()
    cache = tools / "mcplink-downloads"
    cache.mkdir(parents=True, exist_ok=True)
    archive, plugin = [download_asset(cache, *asset) for asset in ASSETS]
    # Rebuild extracted files from verified inputs on every run, so corrupt
    # extracted tools cannot survive an otherwise valid download-cache hit.
    with tempfile.TemporaryDirectory(prefix="mcplink-nsis-", dir=tools) as temporary:
        with zipfile.ZipFile(archive) as source:
            source.extractall(temporary)
        prepared = Path(temporary) / "nsis-3.13"
        additional = prepared / "Plugins/x86-unicode/additional"
        additional.mkdir(parents=True, exist_ok=True)
        shutil.copy2(plugin, additional / plugin.name)
        for name in ("NSIS.exe", "makensis.exe", "Bin/makensis.exe", "Include/MUI2.nsh"):
            if not (prepared / name).is_file():
                raise RuntimeError(f"NSIS archive is missing {name}")
        target = tools / "NSIS"
        if target.is_symlink():
            raise RuntimeError("Refusing to replace a symlinked NSIS directory")
        if target.exists():
            shutil.rmtree(target)
        prepared.replace(target)
    print("Prepared verified NSIS 3.13 and Tauri NSIS plugin 0.5.3", flush=True)


if __name__ == "__main__":
    main()
