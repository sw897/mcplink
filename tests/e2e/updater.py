"""Exercise the desktop's real updater HTTP/signature and first-start CLI code.

Uses loopback releases, ephemeral signing identities and a marked synthetic home.
It does not install a desktop app or contact an upstream MCP service.
"""
import argparse
from contextlib import contextmanager
import copy
import hashlib
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import threading
import time
import urllib.parse

ROOT = Path(__file__).resolve().parents[2]
SIGNER = ROOT / "node_modules/@tauri-apps/cli/tauri.js"


def digest(data):
    return hashlib.sha256(data).hexdigest()


@contextmanager
def windows_directory_writer(path):
    import ctypes
    from ctypes import wintypes

    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.CreateFileW.argtypes = [
        wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD, wintypes.LPVOID,
        wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE,
    ]
    kernel.CreateFileW.restype = wintypes.HANDLE
    kernel.CloseHandle.argtypes = [wintypes.HANDLE]
    kernel.CloseHandle.restype = wintypes.BOOL
    file_add_file = 0x2
    share_read_write = 0x3
    open_existing = 3
    backup_semantics = 0x02000000
    handle = kernel.CreateFileW(str(path), file_add_file, share_read_write, None,
                                open_existing, backup_semantics, None)
    if handle == ctypes.c_void_p(-1).value:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        yield
    finally:
        kernel.CloseHandle(handle)


def signing_identity(home, name="fixture"):
    key = home / (name + ".key")
    result = subprocess.run(["node", str(SIGNER), "signer", "generate", "--ci", "--write-keys", str(key)], capture_output=True, text=True)
    assert result.returncode == 0, "Could not generate ephemeral signing identity"
    key.chmod(0o600)
    return key


def sign(home, key, data, version="0.4.0", name="fixture.bin"):
    path = home / name
    path.write_bytes(data)
    env = {**os.environ, "TAURI_SIGNING_PRIVATE_KEY_PATH": str(key), "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": ""}
    env.pop("TAURI_SIGNING_PRIVATE_KEY", None)
    args = ["node", str(SIGNER), "signer", "sign"]
    if version:
        args.extend(["--app-version", version])
    result = subprocess.run([*args, path.name], cwd=home, env=env, capture_output=True, text=True)
    assert result.returncode == 0, "Could not sign fixture"
    return path.with_name(path.name + ".sig").read_text().strip()


class Releases:
    def __init__(self, port=0):
        self.routes = {}
        self.releases = []
        self.requests = []
        self.pages = None
        self.status = 200
        fixture = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, *_):
                pass

            def do_GET(self):
                fixture.requests.append({"path": self.path, "authorization": "Authorization" in self.headers, "cookie": "Cookie" in self.headers})
                parsed = urllib.parse.urlparse(self.path)
                if parsed.path == "/repos/sw897/mcplink/releases":
                    page = int(urllib.parse.parse_qs(parsed.query)["page"][0])
                    batch = fixture.pages(page) if fixture.pages else fixture.releases
                    body, status, delay, redirect = json.dumps(batch).encode(), fixture.status, 0, None
                else:
                    body, status, delay, redirect = fixture.routes.get(parsed.path, (b"missing", 404, 0, None))
                self.send_response(status)
                if redirect:
                    self.send_header("Location", redirect)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                try:
                    for offset in range(0, len(body), 8192):
                        self.wfile.write(body[offset:offset + 8192])
                        self.wfile.flush()
                        if delay:
                            time.sleep(delay)
                except (BrokenPipeError, ConnectionResetError, ConnectionAbortedError):
                    pass

        self.server = ThreadingHTTPServer(("127.0.0.1", port), Handler)
        self.origin = f"http://127.0.0.1:{self.server.server_port}"
        self.downloads = self.origin + "/sw897/mcplink/releases/download/"
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def publish(self, data, signature, platform="macos", version="0.4.0"):
        target = "darwin-aarch64" if platform == "macos" else "windows-x86_64"
        payload = f"mcplink_update_{version}_aarch64.app.tar.gz" if platform == "macos" else f"mcplink_{version}_x64-setup.exe"
        manifest_name = platform + "-updater.json"
        base = self.downloads + "v" + version + "/"
        metadata = {"version": version, "platforms": {target: {"url": base + payload, "signature": signature, "sha256": digest(data), "size": len(data)}}}
        raw = json.dumps(metadata).encode()
        self.routes = {urllib.parse.urlparse(base + payload).path: (data, 200, 0, None), urllib.parse.urlparse(base + manifest_name).path: (raw, 200, 0, None)}
        assets = [{"name": name, "browser_download_url": base + name, "size": len(content), "state": "uploaded", "digest": "sha256:" + digest(content)} for name, content in ((payload, data), (manifest_name, raw))]
        release = {"tag_name": "v" + version, "name": "Fixture release", "body": "Synthetic update", "published_at": "2026-01-01T00:00:00Z", "draft": False, "prerelease": False, "assets": assets}
        self.releases, self.pages, self.status = [release], None, 200
        return release, metadata, urllib.parse.urlparse(base + manifest_name).path, urllib.parse.urlparse(base + payload).path

    def close(self):
        self.server.shutdown()
        self.server.server_close()


def run(record, live=False):
    home = record / "home"
    home.mkdir(parents=True, mode=0o700)
    (home / ".mcplink-test-fixture").touch()
    key = signing_identity(home)
    other = signing_identity(home, "other")
    data = b"mcplink synthetic update\n" * 16000
    signature = sign(home, key, data)
    subprocess.run(["cargo", "build", "--locked", "-q", "-p", "mcplink-desktop", "--examples"], cwd=ROOT, check=True)
    driver = ROOT / "target/debug/examples" / ("updater_contract.exe" if os.name == "nt" else "updater_contract")
    fixture = Releases()
    args = {"version": "0.3.2", "target": "darwin-aarch64", "home": str(home), "publicKey": str(key) + ".pub", "api": fixture.origin + "/", "downloads": fixture.downloads, "action": "download"}
    scenarios = []
    limits = []

    def call(name, options=None, phase=None, error=None):
        result = subprocess.run([str(driver)], input=json.dumps({**args, **(options or {})}), capture_output=True, text=True, encoding="utf-8", timeout=25)
        assert result.returncode in (0, 2), f"{name}: driver crashed"
        value = json.loads(result.stdout)
        if error:
            assert result.returncode == 2 and error in value.get("error", ""), f"{name}: {value}"
        else:
            assert result.returncode == 0, f"{name}: {value.get('error', 'missing driver error')}"
        if phase:
            assert value.get("phase") == phase, f"{name}: {value}"
        scenarios.append(name)
        return value

    def reset(platform="macos"):
        return fixture.publish(data, signature, platform)

    try:
        reset()
        value = call("signed download and progress", phase="verified")
        assert value["bytes"] == len(data) and value["progressEvents"] > 0
        reset("windows")
        call("Windows manifest and signed NSIS payload contract", {"target": "windows-x86_64"}, phase="verified")
        release, metadata, manifest_path, payload_path = reset()
        fixture.routes[payload_path] = (data, 200, 0.025, None)
        call("cancellable streaming download", {"cancelAfterMs": 60}, error="取消")
        reset()
        fixture.routes[payload_path] = (b"x" + data[1:], 200, 0, None)
        call("changed package rejected before installation", error="SHA-256")
        fixture.publish(data, sign(home, other, data, name="wrong-key.bin"))
        call("wrong signing identity rejected", error="签名")
        fixture.publish(data, sign(home, key, data, version=None, name="unbound.bin"))
        call("signature without signed version rejected", error="版本")
        fixture.publish(data, sign(home, key, data, version="0.3.0", name="replay.bin"))
        call("old signed package cannot impersonate a new version", error="版本")
        release, metadata, manifest_path, payload_path = reset()
        fixture.routes[payload_path] = (data, 302, 0, "http://localhost:1/untrusted")
        call("redirect outside approved origin rejected", error="下载失败")
        release, metadata, manifest_path, payload_path = reset()
        metadata["platforms"]["darwin-aarch64"]["url"] = "https://github.com/another/repo/releases/download/v0.4.0/file"
        fixture.routes[manifest_path] = (json.dumps(metadata).encode(), 200, 0, None)
        call("foreign repository payload rejected", error="地址或大小")
        release, metadata, manifest_path, payload_path = reset()
        release["assets"][0]["size"] += 1
        call("release asset size mismatch rejected", error="地址或大小")
        reset()[0]["assets"][0]["digest"] = "sha256:" + "0" * 64
        call("GitHub asset digest mismatch rejected", error="SHA-256")
        reset()[0]["assets"][1]["browser_download_url"] += "?other=true"
        call("manifest outside exact release path rejected", error="不属于")
        release, _, manifest_path, _ = reset()
        fixture.routes[manifest_path] = (b"not json", 200, 0, None)
        call("malformed update manifest rejected", error="格式")
        reset()[0]["assets"] = []
        call("release missing updater assets is unavailable", phase="unavailable")
        reset()
        call("unsupported platform has no install candidate", {"target": "darwin-x86_64"}, phase="unavailable")
        release = reset()[0]
        release["prerelease"] = True
        call("preview excluded by stable channel", {"preview": False}, phase="no-release")
        call("preview explicitly included", {"preview": True}, phase="verified")
        release = reset()[0]
        release["tag_name"], release["prerelease"] = "v0.5.0-beta.1", False
        call("semver preview cannot bypass stable channel", {"preview": False}, phase="no-release")
        release = reset()[0]
        draft = copy.deepcopy(release)
        draft.update(tag_name="v9.0.0", draft=True)
        malformed = copy.deepcopy(release)
        malformed["tag_name"] = "vv9.0.0"
        fixture.releases.extend([draft, malformed])
        call("drafts and noncanonical tags excluded", phase="verified")
        release = reset()[0]
        release["tag_name"] = "v0.3.2+build.9"
        call("build metadata does not trigger an upgrade", phase="current")
        release = reset()[0]
        fixture.pages = lambda page: [release] * 100 if page == 1 else []
        call("paginated release catalog", phase="verified")
        fixture.pages = lambda _: [release] * 100
        call("bounded catalog never claims current on incomplete history", error="超过检查范围")
        reset()
        fixture.status = 429
        call("rate limit reported as failure", error="限制")
        fixture.status = 500
        call("server failure reported without a false current result", error="HTTP 500")
        assert all(not r["authorization"] and not r["cookie"] for r in fixture.requests), "Updater sent credentials"
        scenarios.append("anonymous requests without MCP credentials or browser cookies")

        executable = "mcplink.exe" if os.name == "nt" else "mcplink"
        target = home / ".cargo/bin" / executable
        target.parent.mkdir(parents=True)
        sidecar = home / "app" / executable
        sidecar.parent.mkdir()
        old, new = b"old synthetic CLI", b"new synthetic CLI"
        sidecar.write_bytes(new)
        sidecar.chmod(0o755)
        receipt = home / "desktop-updates/pending.json"
        receipt.parent.mkdir()

        def pending(checksum=None, cli=target):
            receipt.write_text(json.dumps({"version": "0.4.0", "targets": [{"path": str(cli), "sha256": checksum or digest(old)}]}))

        sync = {"action": "sync", "version": "0.4.0", "sidecar": str(sidecar)}
        target.write_bytes(old)
        pending()
        if os.name == "nt":
            with windows_directory_writer(target.parent):
                synced = call("Windows CLI synchronization with a concurrent directory writer", sync)
        else:
            synced = call("first-start CLI synchronization", sync)
        assert synced["synced"] and target.read_bytes() == new
        assert call("idempotent CLI synchronization", sync)["synced"]
        target.write_bytes(b"user edited")
        call("edited CLI retained", sync, error="已被修改")
        assert target.read_bytes() == b"user edited"
        pending("short")
        call("malformed receipt rejected without panic", sync, error="校验值无效")
        pending()
        target.unlink()
        backup = target.with_name(f".{executable}.mcplink-backup-{digest(old)[:16]}")
        backup.write_bytes(old)
        assert call("interrupted CLI rename restored and completed", sync)["synced"] and target.read_bytes() == new
        pending()
        call("wrong first-start version preserves CLI", {**sync, "version": "0.3.2"}, error="尚未完成")
        assert call("explicitly archive stale update receipt", {**sync, "action": "archive", "version": "0.3.2"})["archived"]
        assert not receipt.exists() and len(list(receipt.parent.glob("pending-abandoned-*.json"))) == 1
        assert not call("archiving does not change the installed CLI", sync)["synced"] and target.read_bytes() == new
        pending()
        target.unlink()
        outside = home / "linked-original"
        outside.write_bytes(old)
        try:
            target.symlink_to(outside)
        except OSError:
            limits.append("OS denied fixture symlink creation; symlink case not executed")
            target.write_bytes(old)
        else:
            call("symlink CLI preserved", sync, error="位置已变化")
            assert outside.read_bytes() == old and target.is_symlink()
        target.unlink()
        target.write_bytes(old)
        alias, real_directory = home / "cli-parent-link", home / "cli-parent-target"
        real_directory.mkdir()
        linked_cli = real_directory / executable
        linked_cli.write_bytes(old)
        if os.name == "nt":
            subprocess.run(["powershell.exe", "-NoProfile", "-NonInteractive", "-Command",
                            "$ErrorActionPreference = 'Stop'; New-Item -ItemType Junction -Path $env:MCPLINK_FIXTURE_JUNCTION -Target $env:MCPLINK_FIXTURE_JUNCTION_TARGET | Out-Null"],
                           env={**os.environ, "MCPLINK_FIXTURE_JUNCTION": str(alias),
                                "MCPLINK_FIXTURE_JUNCTION_TARGET": str(real_directory)},
                           capture_output=True, check=True, timeout=20)
        else:
            alias.symlink_to(real_directory, target_is_directory=True)
        try:
            pending(cli=alias / executable)
            call("linked CLI parent is rejected without replacing its target", sync, error="位置已变化")
            assert linked_cli.read_bytes() == old
        finally:
            if os.name == "nt":
                alias.rmdir()
            else:
                alias.unlink()
        pending()
        stage = target.with_name(f".{executable}.mcplink-update-0.4.0")
        stage.write_bytes(b"unrecognized partial file")
        call("unknown staging file preserved for review", sync, error="临时文件未通过")
        assert target.read_bytes() == old and stage.is_file()
        stage.unlink()
        binary = ROOT / "target/release" / executable
        if not binary.exists():
            binary = ROOT / "target/debug" / executable
        assert binary.is_file(), "Build mcplink-core before updater acceptance"
        version = subprocess.check_output([str(binary), "--version"], text=True).strip().split()[1]
        shutil.copyfile(binary, target)
        shutil.copyfile(binary, sidecar)
        target.chmod(0o755)
        discovery = {"action": "discover", "version": version, "sidecar": str(sidecar)}
        value = call("recognize matching installed bytes without executing the candidate", discovery)
        assert len(value["targets"]) == 1 and Path(value["targets"][0]["path"]) == target
        target.write_bytes(b"unrecognized same-name CLI")
        assert call("unknown same-name CLI is excluded and preserved", discovery)["targets"] == []
        assert target.read_bytes() == b"unrecognized same-name CLI"
        for method, fields, accepted in [
            ("snapshot", {}, True),
            ("connections.probe", {"id": "fixture", "expectedRevision": 1}, True),
            ("connections.probe", {"id": "fixture"}, False),
            ("connections.save", {"confirmedTransferOrigin": "https://fixture.invalid"}, False),
            ("gateway.stop", {}, False),
            ("mcp.request", {}, False),
            ("changes.preview", {"_bridgeExecutable": str(sidecar)}, False),
        ]:
            value = call(f"desktop RPC policy for {method}", {
                "action": "desktop-rpc-contract", "method": method, "params": fields,
            }, error=None if accepted else "桌面" if method != "connections.probe" else "连接修订")
            assert (value.get("accepted") is True) == accepted
        # Exercise the release preparation boundary with a synthetic NSIS payload.
        # This verifies metadata/signatures, not execution of an installer.
        sys.path.insert(0, str(ROOT / "scripts"))
        import prepare_updater
        import upload_release_draft
        candidate = home / "candidate"
        candidate.mkdir()
        setup = candidate / "mcplink_0.4.0_x64-setup.exe"
        setup.write_bytes(data)
        cli = candidate / "mcplink_cli_0.4.0_x86_64-pc-windows-msvc.zip"
        cli.write_bytes(b"synthetic CLI archive")
        manifest = {"version": "0.4.0", "conclusion": "PASS", "revision": "0" * 40, "source_dirty": False, "source_files": {}, "artifacts": {p.name: {"sha256": digest(p.read_bytes()), "bytes": p.stat().st_size} for p in (setup, cli)}}
        (candidate / "release-manifest.json").write_text(json.dumps(manifest))
        verification = {"version": "0.4.0", "publisherSignatureVerified": False, "installed": {"conclusion": "PASS"}, "suites": {name: "PASS" for name in ("agent_cli", "aggregate_gateway", "entry_policy", "catalog_refresh", "cli_workflow", "daemon_lifecycle")}}
        (candidate / "verification.json").write_text(json.dumps(verification))
        signing_env = {**os.environ, "TAURI_SIGNING_PRIVATE_KEY_PATH": str(key), "TAURI_SIGNING_PRIVATE_KEY_PASSWORD": ""}
        signing_env.pop("TAURI_SIGNING_PRIVATE_KEY", None)
        result = subprocess.run([sys.executable, str(ROOT / "scripts/prepare_updater.py"), "--directory", str(candidate), "--platform", "windows", "--fixture-home", str(home), "--public-key", str(key) + ".pub"], cwd=ROOT, env=signing_env, capture_output=True, text=True, timeout=90)
        assert result.returncode == 0, "Release updater preparation failed"
        prepare_updater.verify_update(candidate, "windows", "0.4.0", key=Path(str(key) + ".pub"))
        scenarios.append("release packager produces version-bound platform metadata and signature")
        public_stage = home / "public-staging"
        public_stage.mkdir()
        try:
            upload_release_draft.prepare(candidate, "windows", "v0.4.0", public_stage)
        except ValueError as error:
            assert "signature" in str(error), "Unexpected publication failure"
        else:
            raise AssertionError("Fixture-signed release passed public upload verification")
        assert not list(public_stage.iterdir()), "Untrusted artifacts reached public staging"
        scenarios.append("public upload rejects fixture identity before staging any artifact")
        if live:
            current = json.loads((ROOT / "package.json").read_text())["version"]
            published = call("actual anonymous GitHub published release lookup", {"live": True, "action": "check", "version": current})
            assert published.get("phase") in {"current", "available", "unavailable", "no-release"}, "Live release lookup failed"
    finally:
        fixture.close()
    return {"conclusion": "PASS", "scenarios": scenarios, "limits": limits, "requests": len(fixture.requests), "desktop_installed": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--record", type=Path, required=True)
    parser.add_argument("--live", action="store_true", help="Also check actual public GitHub releases")
    args = parser.parse_args()
    args.record = args.record.resolve()
    args.record.mkdir(parents=True, exist_ok=True, mode=0o700)
    report = {"conclusion": "FAIL"}
    try:
        report = run(args.record, args.live)
    except Exception as error:
        report["error"] = str(error)
        raise
    finally:
        (args.record / "report.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps(report))


if __name__ == "__main__":
    main()
