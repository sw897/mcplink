"""Check published configuration and synthetic credentials in the native Windows stores."""
import ctypes
from ctypes import wintypes
import hashlib
import json
import platform
import sqlite3
import subprocess
import tomllib
import urllib.request
import uuid


CRED_TYPE_GENERIC = 1
CRED_PERSIST_LOCAL_MACHINE = 2
ERROR_NOT_FOUND = 1168
SERVICE = "io.mcplink.credentials"
OTHER_SERVICE = "io.mcplink.synthetic-test"


class Credential(ctypes.Structure):
    _fields_ = [
        ("Flags", wintypes.DWORD), ("Type", wintypes.DWORD), ("TargetName", wintypes.LPWSTR),
        ("Comment", wintypes.LPWSTR), ("LastWritten", wintypes.FILETIME),
        ("CredentialBlobSize", wintypes.DWORD), ("CredentialBlob", ctypes.POINTER(ctypes.c_ubyte)),
        ("Persist", wintypes.DWORD), ("AttributeCount", wintypes.DWORD),
        ("Attributes", ctypes.c_void_p), ("TargetAlias", wintypes.LPWSTR),
        ("UserName", wintypes.LPWSTR),
    ]


class CredentialManager:
    def __init__(self):
        self.native = ctypes.WinDLL("Advapi32", use_last_error=True)
        credential = ctypes.POINTER(Credential)
        self.records = ctypes.POINTER(credential)
        self.native.CredEnumerateW.argtypes = [
            wintypes.LPCWSTR, wintypes.DWORD, ctypes.POINTER(wintypes.DWORD),
            ctypes.POINTER(self.records),
        ]
        self.native.CredEnumerateW.restype = wintypes.BOOL
        self.native.CredWriteW.argtypes = [credential, wintypes.DWORD]
        self.native.CredWriteW.restype = wintypes.BOOL
        self.native.CredDeleteW.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD]
        self.native.CredDeleteW.restype = wintypes.BOOL
        self.native.CredFree.argtypes = [ctypes.c_void_p]
        self.native.CredFree.restype = None

    def targets(self, prefix):
        count, records = wintypes.DWORD(), self.records()
        if not self.native.CredEnumerateW(prefix + "*", 0, ctypes.byref(count), ctypes.byref(records)):
            error = ctypes.get_last_error()
            if error == ERROR_NOT_FOUND:
                return set()
            raise ctypes.WinError(error)
        try:
            return {records[i].contents.TargetName for i in range(count.value)
                    if records[i].contents.Type == CRED_TYPE_GENERIC
                    and records[i].contents.TargetName.startswith(prefix)}
        finally:
            self.native.CredFree(records)

    def seed(self, target):
        assert target not in self.targets(target)
        raw = "synthetic-storage-sentinel".encode("utf-16-le")
        blob = (ctypes.c_ubyte * len(raw)).from_buffer_copy(raw)
        entry = Credential(Type=CRED_TYPE_GENERIC, TargetName=target, CredentialBlobSize=len(raw),
                           CredentialBlob=ctypes.cast(blob, ctypes.POINTER(ctypes.c_ubyte)),
                           Persist=CRED_PERSIST_LOCAL_MACHINE, UserName="mcplink-synthetic-test")
        if not self.native.CredWriteW(ctypes.byref(entry), 0):
            raise ctypes.WinError(ctypes.get_last_error())
        assert target in self.targets(target)

    def delete(self, target):
        if not self.native.CredDeleteW(target, CRED_TYPE_GENERIC, 0):
            error = ctypes.get_last_error()
            if error != ERROR_NOT_FOUND:
                raise ctypes.WinError(error)


def namespace(path):
    return hashlib.sha256(str(path).encode("utf-8")).hexdigest()[:20] + "/"


def powershell_storage(environment, script, path):
    # A PowerShell 7 parent can supply module paths incompatible with Windows PowerShell.
    env = {key: value for key, value in environment.items() if key.casefold() != "psmodulepath"}
    env["MCPLINK_STORAGE_TEST_PATH"] = str(path)
    return subprocess.check_output(
        ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; " + script],
        env=env, text=True, timeout=30,
    ).strip()


def dacl(environment, path):
    command = "(Get-Acl -LiteralPath $env:MCPLINK_STORAGE_TEST_PATH).Sddl"
    return powershell_storage(environment, command, path).split("D:", 1)[1]


def assert_preserved_dacl(before, after):
    before_flags, before_separator, before_rules = before.partition("(")
    after_flags, after_separator, after_rules = after.partition("(")
    # Win32 can mark an ordinary file's DACL as auto-inherited after setting it.
    added_inheritance_marker = before_flags == "" and after_flags == "AI"
    same_rules = (before_separator, before_rules) == (after_separator, after_rules)
    assert same_rules and (before_flags == after_flags or added_inheritance_marker), (
        f"NTFS DACL changed (flags {before_flags!r} to {after_flags!r}, "
        f"access rules unchanged: {same_rules})"
    )


def assert_private(environment, path, current_sid):
    details = powershell_storage(
        environment,
        "$acl=Get-Acl -LiteralPath $env:MCPLINK_STORAGE_TEST_PATH; "
        "@{protected=$acl.AreAccessRulesProtected; rules=@("
        "$acl.GetAccessRules($true,$true,[Security.Principal.SecurityIdentifier]) | "
        "ForEach-Object { @{sid=$_.IdentityReference.Value; inherited=$_.IsInherited; "
        "access=$_.AccessControlType.ToString(); rights=[int]$_.FileSystemRights} })} "
        "| ConvertTo-Json -Depth 4 -Compress",
        path,
    )
    acl = json.loads(details)
    assert acl["protected"], f"Expected protected DACL on {path.name}"
    rules = acl["rules"]
    assert len(rules) == 2 and {rule["sid"] for rule in rules} == {current_sid, "S-1-5-18"}, path.name
    assert all(not rule["inherited"] and rule["access"] == "Allow" and rule["rights"] == 0x1F01FF
               for rule in rules), path.name


def verify_ntfs(h, inject_failure=False):
    config = h.home / ".codex/config.toml"
    connection = h.rpc("connections.save", {"name": "native_acl_fixture", "definition": {
        "url": h.url, "headers": {"Authorization": "synthetic-fixture-authorization"},
    }})["id"]

    def apply():
        _, result = h.changes(connection, ["codex:user"], mode="direct")
        operation, = result["results"]
        return operation["operationId"]

    def restore(operation):
        assert h.rpc("changes.restore", {"operationId": operation})["status"] == "restored"

    inherited = dacl(h.environment, config)
    assert not inherited.startswith("P"), "Expected a config with inherited access rules"
    operation = apply()
    assert_preserved_dacl(inherited, dacl(h.environment, config))
    restore(operation)
    assert_preserved_dacl(inherited, dacl(h.environment, config))
    h.passed("NTFS inherited access rules survive apply and restore")

    powershell_storage(
        h.environment,
        "$p=$env:MCPLINK_STORAGE_TEST_PATH; $acl=Get-Acl -LiteralPath $p; "
        "$acl.SetAccessRuleProtection($true,$true); "
        "$sid=[Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'); "
        "$rule=[Security.AccessControl.FileSystemAccessRule]::new($sid,'Read','Allow'); "
        "$acl.SetAccessRule($rule); Set-Acl -LiteralPath $p -AclObject $acl; "
        "Set-Content -LiteralPath $p -Stream McplinkSyntheticTest -NoNewline -Value 'synthetic-stream'",
        config,
    )

    def acl_and_stream():
        return powershell_storage(
            h.environment, "$p=$env:MCPLINK_STORAGE_TEST_PATH; (Get-Acl -LiteralPath $p).Sddl; "
            "Get-Content -Raw -LiteralPath $p -Stream McplinkSyntheticTest", config,
        )

    before = acl_and_stream()
    assert before.split("D:", 1)[1].startswith("P") and before.endswith("synthetic-stream")
    expected = "intentional ACL mismatch\n" + before if inject_failure else before
    operation = apply()
    assert acl_and_stream() == expected, (
        "Injected NTFS ACL mismatch" if inject_failure else "Native ACL or alternate stream changed on apply"
    )
    user_sid = powershell_storage(
        h.environment, "[Security.Principal.WindowsIdentity]::GetCurrent().User.Value", config,
    )
    backup = h.data / "backups" / (operation + ".config")
    for path in [h.data, h.data / "mcplink.sqlite3", backup.parent, backup]:
        assert_private(h.environment, path, user_sid)
    restore(operation)
    assert acl_and_stream() == before, "Native ACL or alternate stream changed on restore"
    h.passed("NTFS protected DACL and named stream survive apply and restore; backup and database ACLs are private")


def seed_accounts(h):
    token = h.rpc("connections.import", {"sourceId": "hermes:user", "name": "office_docs"})["id"]
    oauth = h.rpc("connections.save", {"name": "upgrade_oauth", "definition": {"url": h.url, "oauth": True}})["id"]
    login = h.rpc("oauth.start", {"id": oauth})
    with urllib.request.urlopen(login["url"], timeout=15) as response:
        assert response.status == 200
    for connection in (token, oauth):
        assert h.rpc("connections.probe", {"id": connection})["connection"]["status"] == "ready"
    return token, oauth


class NativeStorage:
    def __init__(self, harness):
        assert platform.machine().lower() in {"amd64", "x86_64"}, "Windows x64 is required"
        self.harness = harness
        self.harness.environment.pop("MCPLINK_VAULT", None)
        self.manager = CredentialManager()
        self.primary = namespace(harness.data)
        self.separate = namespace(harness.data.with_name("other-synthetic-data"))
        assert not any(self.manager.targets(prefix) for prefix in (self.primary, self.separate))
        self.config = harness.home / ".codex/config.toml"
        self.database = harness.data / "mcplink.sqlite3"

    def native_targets(self, key):
        return {target for target in self.manager.targets(self.primary + key)
                if target.endswith("." + SERVICE)}

    def prepare(self, oauth_id):
        h = self.harness
        assert not (h.data / "fixture-vault").exists()
        self.oauth_id = oauth_id
        self.assert_oauth()
        self.original_config = tomllib.loads(self.config.read_text(encoding="utf-8"))
        connection = h.rpc("connections.save", {"name": "native_storage_fixture", "definition": {
            "url": h.url, "headers": {"Authorization": "synthetic-fixture-authorization"},
        }})["id"]
        _, result = h.changes(connection, ["codex:user"], mode="direct")
        self.previous_operation = result["results"][0]["operationId"]
        payload = {"connectionId": connection, "targetSourceIds": ["codex:user"], "mode": "direct", "action": "disable"}
        self.stale_preview = h.rpc("changes.preview", payload)["planId"]
        self.valid_preview = h.rpc("changes.preview", payload)["planId"]
        assert self.native_targets(f"plan/{self.stale_preview}"), "Baseline did not persist its preview in Credential Manager"
        assert self.native_targets(f"change/{self.previous_operation}")

        self.orphan = f"change/{uuid.uuid4()}"
        self.sentinels = [
            f"{self.separate}plan/{self.stale_preview}.{SERVICE}",
            f"{self.primary}plan/{self.stale_preview}.{OTHER_SERVICE}",
        ]
        for target in [f"{self.primary}{self.orphan}.{SERVICE}", *self.sentinels]:
            self.manager.seed(target)

    def assert_oauth(self):
        assert self.native_targets(f"oauth/{self.oauth_id}"), "Synthetic OAuth record missing from Credential Manager"
        assert not (self.harness.data / "fixture-vault").exists()

    def verify_migration(self):
        h = self.harness
        result = h.rpc("changes.apply", {"planId": self.valid_preview})
        operation, = result["results"]
        assert operation["status"] == "applied", operation.get("message")
        assert h.rpc("changes.restore", {"operationId": operation["operationId"]})["status"] == "restored"

        # The published baseline predates chunking. Use the candidate's native
        # Vault to write a chunked plan, then remove its index to model old storage.
        chunked = h.rpc("connections.save", {"name": "native_chunked_preview_fixture", "definition": {
            "url": h.url, "headers": {"X-Synthetic": "x" * 3100},
        }})["id"]
        chunked_preview = h.rpc("changes.preview", {
            "connectionId": chunked, "targetSourceIds": ["codex:user"], "mode": "direct",
        })["planId"]
        chunked_key = f"plan/{chunked_preview}"
        assert any("/chunks-v1/" in target for target in self.native_targets(chunked_key)), (
            "Expected a native chunked preview"
        )
        with sqlite3.connect(self.database) as db:
            cursor = db.execute("DELETE FROM change_previews WHERE id=?", (chunked_preview,))
            assert cursor.rowcount == 1
            cursor = db.execute("UPDATE settings SET value='0' WHERE name='changes.legacy-sweep'")
            assert cursor.rowcount == 1
        h.rpc("changes.reconcile")
        assert not self.native_targets(f"plan/{self.stale_preview}"), "Expired baseline plan remains"
        assert not self.native_targets(chunked_key), "Legacy-shaped native plan or its chunks remain"
        assert not self.native_targets(self.orphan), "Unreferenced native snapshot remains"
        for target in self.sentinels:
            assert target in self.manager.targets(target), "Sweep crossed a namespace or service boundary"
        assert self.native_targets(f"change/{self.previous_operation}"), "Sweep removed a live undo snapshot"
        assert h.rpc("changes.restore", {"operationId": self.previous_operation})["status"] == "restored"
        assert tomllib.loads(self.config.read_text(encoding="utf-8")) == self.original_config
        with sqlite3.connect(self.database) as db:
            assert db.execute("PRAGMA user_version").fetchone() == (5,)
            assert db.execute("SELECT value FROM settings WHERE name='changes.legacy-sweep'").fetchone() == ("complete",)
        self.assert_oauth()
        h.passed("Credential Manager migrated a live plan, reclaimed expired chunks and orphan, and retained history and other scopes")

    def close(self):
        for prefix in (self.primary, self.separate):
            for target in self.manager.targets(prefix):
                assert target.endswith(("." + SERVICE, "." + OTHER_SERVICE))
                self.manager.delete(target)
            assert not self.manager.targets(prefix)
