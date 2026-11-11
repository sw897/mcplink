"""Exercise signing refusal paths; this does not claim a trusted publisher signature."""
import argparse
import json
import os
from pathlib import Path
import re
import subprocess

from run import BINARY, ROOT


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=BINARY)
    args = parser.parse_args()
    record = args.record.resolve()
    record.mkdir(parents=True, exist_ok=False)
    assert os.name == "nt", "Authenticode requires Windows"
    env = dict(os.environ)
    env.pop("WINDOWS_SIGNING_CERTIFICATE_SHA1", None)
    script = ROOT / "scripts/sign-windows.ps1"
    command = ["powershell.exe", "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(script)]
    rejected = subprocess.run([*command, "-PreflightOnly"], env=env, capture_output=True, text=True, timeout=20)
    assert rejected.returncode != 0, "Missing identity must not produce an unsigned signed-release fallback"
    created = subprocess.run(["powershell.exe", "-NoProfile", "-Command", r"$ErrorActionPreference='Stop'; $c=New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=mcplink isolated signing fixture' -CertStoreLocation Cert:\CurrentUser\My -Provider 'Microsoft Software Key Storage Provider' -KeyExportPolicy NonExportable; $c.Thumbprint"], capture_output=True, text=True, timeout=30)
    assert created.returncode == 0 and re.fullmatch(r"[A-Fa-f0-9]{40}\s*", created.stdout), ("Synthetic identity creation failed", created.returncode, created.stdout, created.stderr)
    thumbprint = created.stdout.strip()
    try:
        env["WINDOWS_SIGNING_CERTIFICATE_SHA1"] = thumbprint
        assert args.binary.is_file(), "Select an existing executable for publisher verification"
        for options in [["-PreflightOnly"], ["-VerifyOnly", "-FilePath", str(args.binary.resolve())]]:
            result = subprocess.run([*command, *options], env=env, capture_output=True, text=True, timeout=30)
            assert result.returncode != 0, "Untrusted identity or unsigned executable was accepted"
    finally:
        subprocess.run(["powershell.exe", "-NoProfile", "-Command", fr"Remove-Item -LiteralPath 'Cert:\CurrentUser\My\{thumbprint}' -DeleteKey -ErrorAction Stop"], check=True, capture_output=True)
    report = {"conclusion": "PASS", "scenarios": ["missing identity fails before building", "self-signed identity cannot pass release preflight", "unsigned or wrong-publisher executable cannot pass signature verification"], "limits": ["Refusal-path verification only; no production certificate or trusted signed build"]}
    (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print("PASS signing refusal paths", record)


if __name__ == "__main__":
    main()
