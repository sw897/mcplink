"""Verify the production CLI rejects the isolated fixture mode before writing credentials."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess


def verify(binary, record):
    record.mkdir(parents=True, exist_ok=False, mode=0o700)
    home, data = record / "home", record / "data"
    home.mkdir()
    (home / ".mcplink-test-fixture").touch()
    binary = binary.resolve(strict=True)
    result = subprocess.run(
        [str(binary), "daemon", "--data-dir", str(data)],
        env={**os.environ, "MCPLINK_HOME": str(home), "MCPLINK_VAULT": "fixture"},
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, encoding="utf-8", timeout=15,
    )
    assert result.returncode and "测试凭据库不可用" in result.stderr, "Production CLI did not refuse fixture Vault"
    assert not (data / "fixture-vault").exists() and not (data / "runtime.json").exists()
    summary = {"conclusion": "PASS", "binary_sha256": hashlib.sha256(binary.read_bytes()).hexdigest(),
               "fixture_vault_created": False, "daemon_started": False}
    (record / "report.json").write_text(json.dumps(summary, indent=2) + "\n")
    print("PASS production CLI refused fixture Vault")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--record", type=Path, required=True)
    args = parser.parse_args()
    verify(args.binary, args.record.resolve())


if __name__ == "__main__":
    main()
