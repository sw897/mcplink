"""Validate the actual DMG, embedded CLI/Skill, architecture and signing state."""
import argparse
from datetime import datetime, timezone
import hashlib
import json
from pathlib import Path
import platform
import plistlib
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from release_manifest import release_version


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def run(command, record, label):
    result = subprocess.run(command, capture_output=True, text=True, timeout=180)
    (record / f"{label}.log").write_text(result.stdout + result.stderr)
    if result.returncode:
        raise AssertionError(f"{label} failed; see {label}.log")
    return result.stdout + result.stderr


def normalized_payload_sha(path):
    # Re-sign disposable copies identically: removing a signature alone leaves a
    # signature-size-dependent __LINKEDIT allocation in the Mach-O header.
    with tempfile.TemporaryDirectory() as directory:
        copy = Path(directory) / path.name
        shutil.copy2(path, copy)
        subprocess.run(["codesign", "--remove-signature", str(copy)], check=True, capture_output=True)
        subprocess.run(["codesign", "--force", "--sign", "-", "--identifier", "io.mcplink.payload-check", "--timestamp=none", str(copy)], check=True, capture_output=True)
        return sha(copy)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--app", type=Path)
    parser.add_argument("--dmg", type=Path)
    parser.add_argument("--require-notarization", action="store_true")
    args = parser.parse_args()
    args.record.mkdir(parents=True, exist_ok=True)
    report = {"executed_at": datetime.now(timezone.utc).isoformat(), "platform": platform.platform(),
              "notarization_checked": args.require_notarization}
    mount = None
    try:
        version = release_version()
        arch = platform.machine()
        dmg_arch = {"arm64": "aarch64", "x86_64": "x64"}[arch]
        app = (args.app or ROOT / "target/release/bundle/macos/mcplink.app").resolve()
        dmg = (args.dmg or ROOT / f"target/release/bundle/dmg/mcplink_{version}_{dmg_arch}.dmg").resolve()
        report.update(version=version, dmg_sha256=sha(dmg))
        run(["hdiutil", "verify", str(dmg)], args.record, "dmg-verify")
        attached = subprocess.check_output(["hdiutil", "attach", "-readonly", "-nobrowse", "-mountrandom", "/Volumes", "-plist", str(dmg)])
        mount = next(Path(e["mount-point"]) for e in plistlib.loads(attached)["system-entities"] if "mount-point" in e)
        installed = mount / "mcplink.app"
        info = plistlib.loads((installed / "Contents/Info.plist").read_bytes())
        assert info["CFBundleIdentifier"] == "io.mcplink.desktop", "Unexpected bundle identifier"
        assert info.get("LSUIElement") is True, "Menu-bar startup must begin without a Dock window"
        assert info["CFBundleShortVersionString"] == version, "Bundle version differs from source"
        files = lambda root: {str(p.relative_to(root)): sha(p) for p in root.rglob("*") if p.is_file()}
        source_files, dmg_files = files(app), files(installed)
        assert source_files == dmg_files, "DMG app differs from the built app"
        report["bundle_files"] = dmg_files
        report["binaries"] = {}
        for name in ("mcplink", "mcplink-desktop"):
            path = installed / "Contents/MacOS" / name
            architectures = subprocess.check_output(["lipo", "-archs", str(path)], text=True).strip().split()
            assert architectures == [arch], f"Unexpected architecture for {name}: {architectures}"
            data = path.read_bytes()
            for prefix in (str(ROOT), str(Path.home())):
                assert prefix.encode() not in data, f"Build-machine path leaked into {name}"
            report["binaries"][name] = {"sha256": sha(path), "architectures": architectures}
        report["build_machine_paths_absent"] = True
        cli = installed / "Contents/MacOS/mcplink"
        schema = json.loads(subprocess.check_output([str(cli), "schema"], text=True))["ok"]
        assert schema["version"] == version and schema["schemaVersion"] == 1, "CLI schema/version differs"
        skill = json.loads(subprocess.check_output([str(cli), "skill", "show"], text=True))["ok"]["content"]
        assert skill == (ROOT / "skills/mcplink/SKILL.md").read_text(), "Embedded Skill is stale"
        report["skill_sha256"] = hashlib.sha256(skill.encode()).hexdigest()
        payload = normalized_payload_sha(cli)
        report["cli_payload_sha256"] = payload
        assert payload == normalized_payload_sha(ROOT / "target/release/mcplink"), "Packaged CLI differs from the latest release build"
        for name in ("LICENSE", "THIRD_PARTY_NOTICES.txt"):
            matches = list((installed / "Contents").rglob(name))
            assert len(matches) == 1 and sha(matches[0]) == sha(ROOT / name), f"Missing/stale {name}"
        run(["codesign", "--verify", "--deep", "--strict", str(installed)], args.record, "codesign-verify")
        signature = run(["codesign", "-d", "--verbose=4", str(installed)], args.record, "codesign-details")
        report["developer_id_signed"] = "Authority=Developer ID Application:" in signature
        report["local_ad_hoc_signature_valid"] = "Signature=adhoc" in signature
        report["notarized"] = False
        if args.require_notarization:
            assert report["developer_id_signed"], "Developer ID signature is required"
            run(["xcrun", "stapler", "validate", str(installed)], args.record, "stapler-app")
            run(["xcrun", "stapler", "validate", str(dmg)], args.record, "stapler-dmg")
            run(["spctl", "--assess", "--type", "execute", "--verbose=4", str(installed)], args.record, "gatekeeper")
            report["notarized"] = True
        report.update(conclusion="PASS", dmg_mount_contents_match=True)
    except Exception as error:
        report.update(conclusion="FAIL", error=str(error))
    finally:
        if mount:
            detached = subprocess.run(["hdiutil", "detach", str(mount)], capture_output=True, text=True)
            if detached.returncode:
                report.update(conclusion="FAIL", detach_error=detached.stderr.strip())
        (args.record / "package-report.json").write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n")
    print(report["conclusion"], report.get("error", ""))
    raise SystemExit(report["conclusion"] != "PASS")


if __name__ == "__main__":
    main()
