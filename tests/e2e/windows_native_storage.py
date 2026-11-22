"""Check published configuration and synthetic credentials in the native Windows stores."""
import ctypes
from ctypes import wintypes
import hashlib
import json
import platform
import sqlite3
import subprocess
import tomllib
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

    def powershell(self, script, path):
        return subprocess.check_output(
            ["powershell.exe", "-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference = 'Stop'; " + script],
            env=self.harness.environment | {"MCPLINK_STORAGE_TEST_PATH": str(path)},
            text=True, timeout=30,
        ).strip()

    def acl_and_stream(self):
        return self.powershell(
            "$p=$env:MCPLINK_STORAGE_TEST_PATH; "
            "(Get-Acl -LiteralPath $p).Sddl; "
            "Get-Content -Raw -LiteralPath $p -Stream McplinkSyntheticTest",
            self.config,
        )

    def assert_private(self, path, current_sid):
        details = self.powershell(
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

    def prepare(self, oauth_id):
        h = self.harness
        assert not (h.data / "fixture-vault").exists()
        self.oauth_id = oauth_id
        self.assert_oauth()
        self.original_config = tomllib.loads(self.config.read_text(encoding="utf-8"))
        connection = h.rpc("connections.save", {"name": "native_storage_fixture", "definition": {
            "url": h.url, "headers": {"Authorization": "synthetic-fixture-authorization", "X-Synthetic": "x" * 3100},
        }})["id"]
        _, result = h.changes(connection, ["codex:user"], mode="direct")
        self.previous_operation = result["results"][0]["operationId"]
        payload = {"connectionId": connection, "targetSourceIds": ["codex:user"], "mode": "direct", "action": "disable"}
        self.stale_preview = h.rpc("changes.preview", payload)["planId"]
        self.valid_preview = h.rpc("changes.preview", payload)["planId"]
        self.stale_targets = self.native_targets(f"plan/{self.stale_preview}")
        assert len(self.stale_targets) > 1, "Baseline did not persist a chunked preview in Credential Manager"
        assert self.native_targets(f"change/{self.previous_operation}")

        self.orphan = f"change/{uuid.uuid4()}"
        self.sentinels = [
            f"{self.separate}plan/{self.stale_preview}.{SERVICE}",
            f"{self.primary}plan/{self.stale_preview}.{OTHER_SERVICE}",
        ]
        for target in [f"{self.primary}{self.orphan}.{SERVICE}", *self.sentinels]:
            self.manager.seed(target)

        self.powershell(
            "$p=$env:MCPLINK_STORAGE_TEST_PATH; $acl=Get-Acl -LiteralPath $p; "
            "$acl.SetAccessRuleProtection($true,$true); "
            "$sid=[Security.Principal.SecurityIdentifier]::new('S-1-5-32-545'); "
            "$rule=[Security.AccessControl.FileSystemAccessRule]::new($sid,'Read','Allow'); "
            "$acl.SetAccessRule($rule); Set-Acl -LiteralPath $p -AclObject $acl; "
            "Set-Content -LiteralPath $p -Stream McplinkSyntheticTest -NoNewline -Value 'synthetic-stream'",
            self.config,
        )
        self.original_ntfs = self.acl_and_stream()
        assert self.original_ntfs.endswith("synthetic-stream")

    def assert_oauth(self):
        assert self.native_targets(f"oauth/{self.oauth_id}"), "Synthetic OAuth record missing from Credential Manager"
        assert not (self.harness.data / "fixture-vault").exists()

    def verify_migration(self, passed):
        h = self.harness
        result = h.rpc("changes.apply", {"planId": self.valid_preview})
        operation, = result["results"]
        assert operation["status"] == "applied", operation.get("message")
        assert self.acl_and_stream() == self.original_ntfs, "Native ACL or alternate stream changed on apply"
        user_sid = self.powershell("[Security.Principal.WindowsIdentity]::GetCurrent().User.Value", self.config)
        backup = h.data / "backups" / (operation["operationId"] + ".config")
        for target in [h.data, self.database, backup.parent, backup]:
            self.assert_private(target, user_sid)
        assert h.rpc("changes.restore", {"operationId": operation["operationId"]})["status"] == "restored"
        assert self.acl_and_stream() == self.original_ntfs, "Native ACL or alternate stream changed on restore"
        passed("NTFS protected DACL and named stream survive apply and restore; backup and database ACLs are private")

        # Advance the grace marker only in this isolated database.
        with sqlite3.connect(self.database) as db:
            cursor = db.execute("UPDATE settings SET value='0' WHERE name='changes.legacy-sweep'")
            assert cursor.rowcount == 1
        h.rpc("changes.reconcile")
        assert not self.native_targets(f"plan/{self.stale_preview}"), "Expired native plan or its chunks remain"
        assert not self.native_targets(self.orphan), "Unreferenced native snapshot remains"
        for target in self.sentinels:
            assert target in self.manager.targets(target), "Sweep crossed a namespace or service boundary"
        assert self.native_targets(f"change/{self.previous_operation}"), "Sweep removed a live undo snapshot"
        assert h.rpc("changes.restore", {"operationId": self.previous_operation})["status"] == "restored"
        assert self.acl_and_stream() == self.original_ntfs, "Native ACL or alternate stream changed on historical restore"
        assert tomllib.loads(self.config.read_text(encoding="utf-8")) == self.original_config
        with sqlite3.connect(self.database) as db:
            assert db.execute("PRAGMA user_version").fetchone() == (5,)
            assert db.execute("SELECT value FROM settings WHERE name='changes.legacy-sweep'").fetchone() == ("complete",)
        self.assert_oauth()
        passed("Credential Manager migrated a live plan, reclaimed expired chunks and orphan, and retained history and other scopes")

    def close(self):
        for prefix in (self.primary, self.separate):
            for target in self.manager.targets(prefix):
                assert target.endswith(("." + SERVICE, "." + OTHER_SERVICE))
                self.manager.delete(target)
            assert not self.manager.targets(prefix)
