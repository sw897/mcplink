"""Package the Windows CLI with normalized ZIP metadata and required licenses."""
import argparse
from pathlib import Path
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    files = [(args.binary, "mcplink.exe"), (ROOT / "LICENSE", "LICENSE"), (ROOT / "THIRD_PARTY_NOTICES.txt", "THIRD_PARTY_NOTICES.txt")]
    with zipfile.ZipFile(args.output, "x", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path, name in files:
            info = zipfile.ZipInfo(name)
            info.create_system = 0
            info.external_attr = 0
            info.compress_type = zipfile.ZIP_DEFLATED
            archive.writestr(info, path.read_bytes())
    with zipfile.ZipFile(args.output) as archive:
        assert set(archive.namelist()) == {name for _, name in files}
        for path, name in files:
            assert archive.read(name) == path.read_bytes()
            assert not archive.getinfo(name).extra
    print("Packaged Windows CLI with normalized metadata and licenses")


if __name__ == "__main__":
    main()
