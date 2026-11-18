"""Explicitly create one persistent local signing identity; never change system trust."""
import base64
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[1]


def run(args, value=None, **options):
    result = subprocess.run(args, input=value, capture_output=True, **options)
    if result.returncode:
        if Path(args[0]).name == "macos-identity":
            status = json.loads(result.stdout).get("status")
            raise RuntimeError(f"Keychain identity import failed (OSStatus {status})")
        raise RuntimeError(f"Signing setup command failed: {Path(args[0]).name} (exit {result.returncode})")
    return result.stdout


def main():
    if sys.platform != "darwin" or sys.argv[1:] != ["--create-local-identity"]:
        raise SystemExit("Usage (macOS only): python3 scripts/setup_macos_signing.py --create-local-identity")
    os.umask(0o077)
    local = ROOT / ".local"
    local.mkdir(exist_ok=True)
    config = local / "macos-signing.json"
    if config.exists():
        raise SystemExit("Local signing configuration already exists; reuse it or review it explicitly before replacement")
    openssl = shutil.which("openssl")
    if not openssl:
        raise SystemExit("OpenSSL is required to prepare the local certificate")
    helper = local / "bin/macos-identity"
    helper.parent.mkdir(exist_ok=True)
    run(["/usr/bin/swiftc", str(ROOT / "scripts/macos_identity.swift"), "-o", str(helper)])
    private = run([openssl, "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:3072"])
    label = "mcplink Local Development"
    certificate = run([openssl, "req", "-new", "-x509", "-sha256", "-days", "3650", "-key", "/dev/stdin",
                       "-subj", "/CN=" + label, "-addext", "basicConstraints=critical,CA:FALSE",
                       "-addext", "keyUsage=critical,digitalSignature", "-addext", "extendedKeyUsage=codeSigning"], private)
    fingerprint = run([openssl, "x509", "-noout", "-fingerprint", "-sha1"], certificate).decode().strip().split("=", 1)[1].replace(":", "")
    password = secrets.token_urlsafe(36)
    read_fd, write_fd = os.pipe()
    certificate_fd, certificate_write = os.pipe()
    try:
        os.write(write_fd, (password + "\n").encode()); os.close(write_fd); write_fd = -1
        os.write(certificate_write, certificate); os.close(certificate_write); certificate_write = -1
        archive = run([openssl, "pkcs12", "-export", "-inkey", "/dev/stdin", "-in", f"/dev/fd/{certificate_fd}",
                       "-name", label, "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1",
                       "-passout", f"fd:{read_fd}"], private, pass_fds=(read_fd, certificate_fd))
    finally:
        os.close(read_fd)
        os.close(certificate_fd)
        if write_fd != -1:
            os.close(write_fd)
        if certificate_write != -1:
            os.close(certificate_write)
    payload = json.dumps({"pkcs12": base64.b64encode(archive).decode(), "password": password}).encode()
    result = json.loads(run([str(helper), "--import-identity"], payload))
    if not result.get("imported"):
        raise RuntimeError("Local identity was not imported")
    # Only the public selector is persisted. No PEM/P12/private key/password files.
    config.write_text(json.dumps({"identity": fingerprint, "label": label, "scope": "local development only",
                                  "private_key_store": "current-user Keychain", "system_trust_modified": False}, indent=2) + "\n")
    print(json.dumps({"created": True, "config": str(config), "private_key_store": "Keychain", "system_trust_modified": False}))


if __name__ == "__main__":
    main()
