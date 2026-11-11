"""Add verified artifacts to a draft without replacing files or publishing it."""
import argparse
import json
from pathlib import Path
import subprocess
import shutil
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tag", required=True)
    parser.add_argument("--directory", type=Path, required=True)
    parser.add_argument("--platform", choices=["macos", "windows"], required=True)
    args = parser.parse_args()
    artifacts = sorted(path for path in args.directory.iterdir() if path.is_file())
    assert artifacts, "No verified release artifacts"
    staging = tempfile.TemporaryDirectory(prefix="mcplink-release-metadata-")
    prepared = []
    for path in artifacts:
        if path.suffix in {".json", ".md"} or path.name == "SHA256SUMS":
            target = Path(staging.name) / (args.platform + "-" + path.name)
            shutil.copyfile(path, target)
            prepared.append(target)
        else:
            prepared.append(path)
    artifacts = prepared
    result = subprocess.run(["gh", "release", "view", args.tag, "--json", "isDraft,tagName"], capture_output=True, text=True)
    if result.returncode == 0:
        release = json.loads(result.stdout)
        assert release["isDraft"] and release["tagName"] == args.tag, "Only an existing matching draft may receive artifacts"
        subprocess.run(["gh", "release", "upload", args.tag, *map(str, artifacts)], check=True)
    else:
        subprocess.run(["gh", "release", "create", args.tag, *map(str, artifacts), "--verify-tag", "--draft", "--title", f"mcplink {args.tag}", "--notes-file", "docs/release-notes.md"], check=True)


if __name__ == "__main__":
    main()
