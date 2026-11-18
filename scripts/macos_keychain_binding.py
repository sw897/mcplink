"""Nonsecret fixed-helper metadata, shared by prepare/deploy/verification tools."""
from contextlib import contextmanager
import fcntl
import hashlib
import json
import os
from pathlib import Path
import stat


def private_directory(path, create=False):
    try:
        info = path.lstat()
    except FileNotFoundError:
        if not create:
            return False
        path.mkdir(mode=0o700)
        info = path.lstat()
    assert stat.S_ISDIR(info.st_mode) and not info.st_mode & 0o022, "Unsafe helper directory"
    return True


def read_regular(path, limit):
    try:
        info = path.lstat()
    except FileNotFoundError:
        return None
    assert stat.S_ISREG(info.st_mode) and info.st_size <= limit, "Helper metadata must be a bounded regular file; no fallback"
    with os.fdopen(os.open(path, os.O_RDONLY | os.O_NOFOLLOW), "rb") as stream:
        assert stat.S_ISREG(os.fstat(stream.fileno()).st_mode)
        value = stream.read(limit + 1)
    assert len(value) <= limit, "Helper file too large"
    return value


def active_helper(data):
    root = data / "keychain-helper"
    if not private_directory(root):
        return None
    raw = read_regular(root / "active.json", 4096)
    if raw is None:
        return None
    def unique_object(pairs):
        value = {}
        for key, item in pairs:
            assert key not in value, "Duplicate helper binding field"
            value[key] = item
        return value
    binding = json.loads(raw, object_pairs_hook=unique_object)
    assert set(binding) == {"version", "sha256"} and type(binding["version"]) is int and binding["version"] in (1, 2), "Invalid helper binding"
    digest = binding["sha256"]
    assert isinstance(digest, str) and len(digest) == 64 and all(c in "0123456789abcdef" for c in digest), "Invalid helper pin"
    path = root / "mcplink-keychain"
    if binding["version"] == 2:
        assert private_directory(root / "versions") and private_directory(root / "versions" / digest), "Missing helper version directory"
        path = root / "versions" / digest / "mcplink-keychain"
    helper = read_regular(path, 128 * 1024 * 1024)
    assert helper is not None and hashlib.sha256(helper).hexdigest() == digest, "Fixed helper changed; review separate migration"
    return {"path": path, "version": binding["version"], "sha256": digest,
            "binding_sha256": hashlib.sha256(raw).hexdigest()}


@contextmanager
def lifecycle_lock(data):
    assert data.is_absolute() and not data.is_symlink(), "Keep the exact data-directory namespace"
    assert private_directory(data), "Missing data directory"
    root = data / "keychain-helper"
    private_directory(root, create=True)
    descriptor = os.open(root / ".lifecycle.lock", os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        assert stat.S_ISREG(os.fstat(descriptor).st_mode), "Invalid lifecycle lock"
        # Same flock as the native updater/migrate/recover commands. Never wait
        # silently while a different lifecycle transaction may need user input.
        fcntl.flock(descriptor, fcntl.LOCK_EX | fcntl.LOCK_NB)
        yield
    finally:
        os.close(descriptor)
