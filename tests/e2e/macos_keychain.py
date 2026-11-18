"""Real Keychain worker acceptance with a separate disposable Keychain and signers."""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import secrets
import shutil
import subprocess
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
from setup_macos_signing import run


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--binary", type=Path, default=ROOT / "target/debug/mcplink")
    args = parser.parse_args()
    assert sys.platform == "darwin"
    os.umask(0o077)
    record = args.record.resolve(); record.mkdir(parents=True, exist_ok=False)
    data = record / "data"; data.mkdir()
    (data / ".mcplink-keychain-test-fixture").touch()
    keychain = data / "fixture.keychain-db"
    audit = record / "identity"
    run(["swiftc", str(ROOT / "scripts/macos_identity.swift"), "-o", str(audit)])
    password = secrets.token_urlsafe(32)
    passed = []
    report = {"conclusion": "FAIL", "existing_credentials_touched": False, "keychain_ui_allowed": False}
    created = False
    try:
        run([str(audit), "--create-fixture-keychain", str(keychain)], json.dumps({"password": password}).encode())
        created = True
        private = run(["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"])
        cert = run(["openssl", "req", "-new", "-x509", "-sha256", "-days", "2", "-key", "/dev/stdin", "-subj", "/CN=mcplink isolated fixture",
                    "-addext", "basicConstraints=critical,CA:FALSE", "-addext", "keyUsage=critical,digitalSignature", "-addext", "extendedKeyUsage=codeSigning"], private)
        fingerprint = run(["openssl", "x509", "-noout", "-fingerprint", "-sha1"], cert).decode().strip().split("=", 1)[1].replace(":", "")
        cert_fd, cert_write = os.pipe(); pass_fd, pass_write = os.pipe()
        try:
            os.write(cert_write, cert); os.close(cert_write)
            os.write(pass_write, (password + "\n").encode()); os.close(pass_write)
            archive = run(["openssl", "pkcs12", "-export", "-inkey", "/dev/stdin", "-in", f"/dev/fd/{cert_fd}",
                           "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1", "-passout", f"fd:{pass_fd}"], private,
                          pass_fds=(cert_fd, pass_fd))
        finally:
            os.close(cert_fd); os.close(pass_fd)
        payload = json.dumps({"pkcs12": base64.b64encode(archive).decode(), "password": password}).encode()
        run([str(audit), "--import-identity", "--fixture-keychain", str(keychain)], payload)
        # Only the newly created disposable signing key is affected. The known
        # synthetic password avoids any system dialog; login Keychain untouched.
        run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", password, str(keychain)])

        def sign(path, identifier):
            run(["codesign", "--force", "--keychain", str(keychain), "--sign", fingerprint,
                 "--identifier", identifier, "--options", "runtime", "--timestamp=none", str(path)])

        helper = record / "mcplink-keychain"
        run(["swiftc", "-O", str(ROOT / "scripts/macos_keychain.swift"), "-o", str(helper)])
        sign(helper, "io.mcplink.keychain")
        helper_hash = hashlib.sha256(helper.read_bytes()).hexdigest()
        callers = []
        for index, identifier in enumerate(("io.mcplink.cli", "io.mcplink.desktop", "io.mcplink.untrusted")):
            source = record / f"caller-{index}.swift"
            source.write_text((ROOT / "tests/fixtures/keychain_client.swift").read_text().replace("MCPLINK_FIXTURE_BUILD", f"build-{index}"))
            caller = record / f"caller-{index}"
            run(["swiftc", "-O", str(source), "-o", str(caller)])
            sign(caller, identifier)
            callers.append(caller)
        audit_result = json.loads(run([str(audit), *map(str, callers)]))
        assert len({s["cdhash"] for s in audit_result["signatures"]}) == 3
        passed.append("three distinct signed callers with one fixed helper")

        def request(caller, operation, value=None, extra=None, expected=0):
            obj = {"version": 1, "operation": operation, "dataDirectory": str(data), "key": "definition/synthetic"}
            if value is not None: obj["value"] = value
            obj.update(extra or {})
            result = subprocess.run([str(caller), str(helper)], input=json.dumps(obj), capture_output=True, text=True, timeout=20)
            assert result.returncode == expected, f"fixture request exit {result.returncode}"
            response = json.loads(result.stdout)
            return response

        assert request(callers[0], "get")["found"] is False
        sentinel = "synthetic-only-" + secrets.token_hex(12)
        assert request(callers[0], "set", sentinel)["ok"]
        assert request(callers[1], "get")["value"] == sentinel
        passed.append("different caller builds reuse the same Keychain worker grant")
        assert request(callers[1], "set", sentinel + "-updated")["ok"]
        assert request(callers[0], "get")["value"] == sentinel + "-updated"
        assert request(callers[2], "get", expected=1)["error"] == "caller_identity"
        direct = subprocess.run([str(helper)], input="{}", capture_output=True, text=True, timeout=20)
        assert direct.returncode == 1 and json.loads(direct.stdout)["error"] == "caller_identity"
        passed.append("incorrect and unsigned parent identities rejected before Keychain access")
        assert request(callers[0], "get", extra={"key": "../escape"}, expected=1)["error"] == "invalid_request"
        assert request(callers[0], "authorize", expected=1)["error"] == "invalid_request"
        passed.append("ordinary protocol cannot enable consent UI or escape account scope")
        assert request(callers[0], "delete")["ok"]
        assert request(callers[1], "get")["found"] is False
        assert hashlib.sha256(helper.read_bytes()).hexdigest() == helper_hash
        passed.append("update/delete round trip preserves fixed helper bytes")

        # The actual Rust CLI/daemon, with fixture-native configs but the real
        # helper rather than MCPLINK_VAULT=fixture.
        core = record / "mcplink"
        shutil.copy2(args.binary, core); sign(core, "io.mcplink.cli")
        home = record / "home"; home.mkdir()
        for path, content in ((".codex/config.toml", ""), (".hermes/config.yaml", "{}"), (".claude.json", "{}")):
            target = home / path; target.parent.mkdir(parents=True, exist_ok=True); target.write_text(content)
        env = {**os.environ, "MCPLINK_HOME": str(home), "MCPLINK_DATA_DIR": str(data)}
        env.pop("MCPLINK_VAULT", None)
        installed = data / "keychain-helper"; installed.mkdir()
        shutil.copy2(helper, installed / "mcplink-keychain")
        (installed / "active.json").write_text(json.dumps({"version": 1, "sha256": helper_hash}))
        def cli(*argv, expected=0):
            result = subprocess.run([str(core), "--no-auth-ui", *argv], env=env, capture_output=True, text=True, timeout=30)
            assert result.returncode == expected, f"Rust CLI exit {result.returncode}"
            return json.loads(result.stdout)
        try:
            assert "ok" in cli("status")
            assert cli("connections", "save", "--stdin", expected=2)["error"]["code"] == "invalid_input"
            preview = cli("keychain", "preview", "--helper", str(helper))["ok"]
            assert preview["credentialValuesRead"] is False and len(preview["items"]) == 1
            binding_bytes = (installed / "active.json").read_bytes()
            changed_plan = subprocess.run([str(core), "keychain", "authorize", "--helper", str(helper),
                                           "--preview-hash", "0" * 64, "--confirm"],
                                          env=env, capture_output=True, text=True, timeout=20)
            assert changed_plan.returncode == 1 and (installed / "active.json").read_bytes() == binding_bytes
            assert cli("keychain", "authorize", "--helper", str(helper), "--preview-hash", preview["previewHash"], "--confirm", expected=1)["error"]
            cli("gateway", "stop")
            binding = installed / "active.json"
            binding.write_text(json.dumps({"version": 1, "sha256": "0" * 64}))
            assert "error" in cli("status", expected=1)
            passed.append("actual Rust daemon uses helper; wrong pin and no-UI authorization fail closed")
        finally:
            subprocess.run([str(core), "--no-auth-ui", "gateway", "stop"], env=env, capture_output=True, timeout=30)
        report.update(conclusion="PASS", passed=passed, helper_sha256=helper_hash)
    except Exception as error:
        report.update(error_class=type(error).__name__, passed=passed)
        raise
    finally:
        if created:
            run([str(audit), "--delete-fixture-keychain", str(keychain)])
        report["temporary_keychain_removed"] = not keychain.exists()
        (record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
        print(json.dumps(report))


if __name__ == "__main__":
    main()
