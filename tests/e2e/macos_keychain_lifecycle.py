"""Failure/recovery cases through the actual CLI and production updater preflight.

All credentials, keys and partition edits below belong to the disposable fixture.
No login Keychain, installed helper or real Agent configuration is accessed.
"""
import hashlib
import gzip
import io
import json
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import uuid

ROOT = Path(__file__).resolve().parents[2]


def exercise(record, data, keychain, password, audit, helper, core, env, sign, cli, run, passed):
    from macos_keychain_binding import active_helper

    installed = data / "keychain-helper"
    binding = installed / "active.json"
    original = binding.read_bytes()
    original_hash = hashlib.sha256(helper.read_bytes()).hexdigest()
    original_file = installed / "mcplink-keychain"
    cli("gateway", "stop")

    # Binding absence is different from a broken link, directory or invalid JSON.
    binding.unlink()
    binding.symlink_to(installed / "missing-binding")
    assert cli("status", expected=1)["error"]
    try:
        active_helper(data)
        raise AssertionError("Python metadata reader accepted a dangling binding")
    except AssertionError as error:
        assert "bounded regular" in str(error)
    binding.unlink(); binding.mkdir()
    assert cli("status", expected=1)["error"]
    binding.rmdir(); binding.write_text('{"version":3,"sha256":"' + original_hash + '"}')
    assert cli("status", expected=1)["error"]
    binding.write_bytes(original)
    passed.append("dangling, non-file and unsupported bindings fail closed in native/Python paths")

    # A changed helper is a separate immutable candidate, not a main rebuild.
    source = record / "helper-next.swift"
    source.write_text((ROOT / "scripts/macos_keychain.swift").read_text().replace('"invalid_request"', '"invalid_request_lifecycle_fixture"'))
    candidate = record / "helper-next"
    run(["swiftc", "-O", "-D", "MCPLINK_TEST_FIXTURES", str(source), "-o", str(candidate)])
    sign(candidate, "io.mcplink.keychain")
    next_hash = hashlib.sha256(candidate.read_bytes()).hexdigest()
    assert next_hash != original_hash
    plan = cli("keychain", "migrate-preview", "--helper", str(candidate))["ok"]
    assert not plan["credentialValuesRead"] and not plan["credentialsMoved"]
    migration = ("keychain", "migrate", "--helper", str(candidate), "--preview-hash", plan["previewHash"], "--confirm")
    signatures = json.loads(run([str(audit), str(helper), str(candidate)]))["signatures"]
    account = hashlib.sha256(str(data).encode()).hexdigest()[:20] + "/definition/runtime-auth-v1"
    # Lock only the disposable Keychain for a deterministic access refusal.
    # Fresh fixture grants can otherwise be cached by designated requirement.
    run(["security", "lock-keychain", str(keychain)])
    try:
        assert cli(*migration, expected=1)["error"]
    finally:
        run(["security", "unlock-keychain", "-p", password, str(keychain)])
    assert binding.read_bytes() == original
    assert not (installed / "rollback.json").exists()
    staged = installed / "versions" / next_hash / "mcplink-keychain"
    assert hashlib.sha256(staged.read_bytes()).hexdigest() == next_hash
    assert cli("keychain", "migrate", "--helper", str(candidate), "--preview-hash", "0" * 64, "--confirm", expected=1)["error"]
    passed.append("denied or stale migration keeps the original binding and grants, retaining the complete candidate")

    driver = record / "updater-contract"
    shutil.copy2(ROOT / "target/debug/examples/updater_contract", driver)
    sign(driver, "io.mcplink.cli")
    (record / ".mcplink-test-fixture").touch()
    app = record / "mcplink.app"
    macos = app / "Contents/MacOS"; macos.mkdir(parents=True)

    # A second disposable certificate exercises certificate mismatch separately
    # from ad-hoc signatures. Its private material remains in unlinked pipes.
    private = run(["openssl", "genpkey", "-algorithm", "RSA", "-pkeyopt", "rsa_keygen_bits:2048"])
    cert = run(["openssl", "req", "-new", "-x509", "-sha256", "-days", "2", "-config", str(record / "fixture-openssl.cnf"),
                "-subj", "/CN=mcplink alternate fixture", "-key", "/dev/stdin"], private)
    fingerprint = run(["openssl", "x509", "-noout", "-fingerprint", "-sha1"], cert).decode().strip().split("=", 1)[1].replace(":", "")
    with tempfile.TemporaryFile() as cert_stream, tempfile.TemporaryFile() as pass_stream:
        cert_stream.write(cert); cert_stream.seek(0)
        pass_stream.write((password + "\n").encode()); pass_stream.seek(0)
        archive = run(["openssl", "pkcs12", "-export", "-inkey", "/dev/stdin", "-in", f"/dev/fd/{cert_stream.fileno()}",
                       "-name", "mcplink alternate fixture", "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES",
                       "-macalg", "sha1", "-passout", f"fd:{pass_stream.fileno()}"], private,
                      pass_fds=(cert_stream.fileno(), pass_stream.fileno()))
    with tempfile.TemporaryFile() as stream:
        stream.write(archive); stream.seek(0)
        run(["security", "import", f"/dev/fd/{stream.fileno()}", "-f", "pkcs12", "-k", str(keychain),
             "-P", password, "-T", "/usr/bin/codesign"], pass_fds=(stream.fileno(),))
    run(["security", "set-key-partition-list", "-S", "apple-tool:,apple:", "-s", "-k", password, str(keychain)])

    def other_sign(path, identifier):
        run(["codesign", "--force", "--keychain", str(keychain), "--sign", fingerprint, "--identifier", identifier,
             "--options", "runtime", "--timestamp=none", str(path)])

    def package(label, capability=2, adhoc=False, wrong_cli=False, other_certificate=False):
        (app / "Contents/Info.plist").write_bytes(plistlib.dumps({
            "CFBundleIdentifier": "io.mcplink.desktop", "CFBundleExecutable": "mcplink-desktop",
            "CFBundleVersion": "0.3.2", "MCPLinkKeychainBindingVersion": capability,
        }))
        shutil.copy2(core, macos / "mcplink")
        shutil.copy2(record / "caller-1", macos / "mcplink-desktop")
        if wrong_cli:
            sign(macos / "mcplink", "io.mcplink.untrusted")
        if adhoc:
            for path in (macos / "mcplink", macos / "mcplink-desktop", app):
                run(["codesign", "--force", "--sign", "-", str(path)])
        else:
            if other_certificate:
                other_sign(macos / "mcplink", "io.mcplink.cli")
                other_sign(macos / "mcplink-desktop", "io.mcplink.desktop")
                other_sign(app, "io.mcplink.desktop")
            else:
                sign(app, "io.mcplink.desktop")
        archive = record / (label + ".tar.gz")
        with tarfile.open(archive, "w:gz", compresslevel=1) as stream:
            stream.add(app, arcname="mcplink.app")
        return archive

    def preflight(archive, expected=0, hold=None, target_data=data):
        args = {"action": "keychain-update-preflight", "home": str(record), "data": str(target_data), "package": str(archive)}
        if hold:
            args["holdMs"] = hold
            process = subprocess.Popen([str(driver)], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, env=env)
            process.stdin.write(json.dumps(args)); process.stdin.close()
            assert json.loads(process.stdout.readline()) == {"locked": True}
            return process
        result = subprocess.run([str(driver)], input=json.dumps(args), env=env, capture_output=True, text=True, timeout=30)
        assert result.returncode == expected, f"Updater preflight: {result.stdout}"
        return json.loads(result.stdout)

    compatible = package("same-certificate")
    unbound_data = record / "no-helper-data"
    unbound_data.mkdir()
    assert preflight(compatible, target_data=unbound_data)["verified"]
    incomplete = record / "incomplete.app.tar.gz"
    with tarfile.open(incomplete, "w:gz"):
        pass
    assert preflight(incomplete, 2, target_data=unbound_data)["error"]
    passed.append("macOS archive preflight validates app structure even without a fixed helper")
    assert preflight(compatible)["verified"]
    assert preflight(package("ad-hoc", adhoc=True), 2)["error"]
    assert preflight(package("wrong-cli", wrong_cli=True), 2)["error"]
    assert preflight(package("other-certificate", other_certificate=True), 2)["error"]
    for label, name, kind in (("traversal", "../escape", tarfile.REGTYPE),
                              ("absolute", "/mcplink.app/escape", tarfile.REGTYPE),
                              ("symlink", "mcplink.app/link", tarfile.SYMTYPE),
                              ("hardlink", "mcplink.app/link", tarfile.LNKTYPE),
                              ("special", "mcplink.app/pipe", tarfile.FIFOTYPE)):
        archive = record / (label + ".tar.gz")
        with tarfile.open(archive, "w:gz") as stream:
            entry = tarfile.TarInfo(name); entry.type = kind
            if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE): entry.linkname = "/tmp/escape"
            stream.addfile(entry, io.BytesIO())
        assert preflight(archive, 2)["error"]
    duplicate = record / "duplicate.tar.gz"
    with tarfile.open(duplicate, "w:gz") as stream:
        for _ in range(2): stream.addfile(tarfile.TarInfo("mcplink.app/duplicate"), io.BytesIO())
    assert preflight(duplicate, 2)["error"]
    oversized = record / "oversized.tar.gz"
    with gzip.open(oversized, "wb") as stream:
        entry = tarfile.TarInfo("mcplink.app/oversized"); entry.size = 1024 * 1024 * 1024 + 1
        stream.write(entry.tobuf())  # Rejected from its declared size before payload extraction.
    assert "范围过大" in preflight(oversized, 2)["error"]
    assert not (record.parent / "escape").exists() and binding.read_bytes() == original
    passed.append("production updater accepts compatible code and rejects ad-hoc, other-certificate, wrong-role and unsafe archives before replacement")

    held = preflight(compatible, hold=4000)
    try:
        assert cli(*migration, expected=1)["error"]
        assert binding.read_bytes() == original
    finally:
        assert held.wait(timeout=10) == 0
    passed.append("native updater and helper lifecycle share an exclusive lock")

    # Explicit fixture-only grant: preserve both exact cdhash partitions. The
    # known synthetic password and explicit fixture path cannot touch user items.
    assert keychain == data / "fixture.keychain-db" and (data / ".mcplink-keychain-test-fixture").is_file()
    lookup = ["-s", "io.mcplink.credentials", "-a", account]
    found = subprocess.run(["security", "find-generic-password", *lookup, str(keychain)],
                           capture_output=True, timeout=20).returncode
    assert found == 0, f"Runtime root metadata lookup in the fixture Keychain failed (exit {found})"
    partitions = ",".join("cdhash:" + value["cdhash"] for value in signatures)
    result = subprocess.run(["security", "set-generic-password-partition-list", *lookup,
                             "-S", partitions, "-k", password, str(keychain)],
                            capture_output=True, text=True, timeout=20)
    assert result.returncode == 0, "Fixture partition update failed: " + result.stderr.replace(password, "[fixture password]").strip()

    result = cli(*migration)["ok"]
    assert result["bindingVersion"] == 2 and result["previousHelperPreserved"]
    assert active_helper(data)["path"] == staged
    active = binding.read_bytes()
    receipt = (installed / "rollback.json").read_bytes()
    assert json.loads(receipt)["previous_json"].encode() == original
    assert hashlib.sha256(original_file.read_bytes()).hexdigest() == original_hash
    assert preflight(compatible)["verified"]
    assert preflight(package("old-binding-format", capability=1), 2)["error"]
    assert "ok" in cli("status")
    cli("gateway", "stop")
    passed.append("pre-granted no-UI migration switches atomically, retains old bytes and rejects incompatible binding versions")

    recovery = cli("keychain", "recover-preview")["ok"]
    assert recovery["action"] == "rollback" and not recovery["credentialValuesRead"]
    assert cli("keychain", "recover", "--preview-hash", "0" * 64, "--confirm", expected=1)["error"]
    assert binding.read_bytes() == active
    assert cli("keychain", "recover", "--preview-hash", recovery["previewHash"], "--confirm")["ok"]["action"] == "rollback"
    assert binding.read_bytes() == original and staged.is_file() and original_file.is_file()
    # Simulate power loss after receipt fsync but before active pointer commit.
    (installed / "rollback.json").write_bytes(receipt)
    pending = cli("keychain", "recover-preview")["ok"]
    assert pending["action"] == "abort-pending"
    assert cli("keychain", "recover", "--preview-hash", pending["previewHash"], "--confirm")["ok"]
    assert binding.read_bytes() == original and not (installed / "rollback.json").exists()
    passed.append("crashes before/after pointer commit recover without rewriting credentials or either helper")

    # A legacy partial first install and abandoned temp are quarantined only
    # after a reviewed plan. The shared atomic installer was exercised above.
    binding.unlink()
    original_file.write_bytes(b"synthetic-interrupted-install")
    assert cli("keychain", "preview", "--helper", str(helper), expected=1)["error"]
    orphan = installed / (".install-" + str(uuid.uuid4()) + ".tmp")
    orphan.write_bytes(b"synthetic-partial-stage")
    recovery = cli("keychain", "recover-preview")["ok"]
    assert recovery["action"] == "quarantine-unbound" and len(recovery["stagingFiles"]) == 1
    assert cli("keychain", "recover", "--preview-hash", recovery["previewHash"], "--confirm")["ok"]["quarantinedFiles"] == 2
    assert not original_file.exists() and not orphan.exists()
    assert sorted(p.read_bytes() for p in (installed / "recovered").iterdir()) == sorted([b"synthetic-interrupted-install", b"synthetic-partial-stage"])
    assert cli("keychain", "preview", "--helper", str(helper))["ok"]["action"] == "authorize"
    shutil.copy2(helper, original_file); binding.write_bytes(original)
    assert "ok" in cli("status")
    cli("gateway", "stop")
    passed.append("interrupted initial installation has explicit byte-preserving quarantine and can be retried")

    # Shared deployment tools must resolve v2 exactly, never overwrite versioned
    # helpers or silently treat broken binding metadata as absent.
    binding.write_bytes(active)
    assert active_helper(data)["sha256"] == next_hash
    binding.write_bytes(original)
    assert hashlib.sha256(original_file.read_bytes()).hexdigest() == original_hash
