"""Package macOS app/CLI without local ownership or extended metadata."""
import argparse
import gzip
from pathlib import Path
import stat
import tarfile
import zipfile

ROOT = Path(__file__).resolve().parents[1]


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--app", type=Path, required=True)
    parser.add_argument("--zip", type=Path, required=True)
    parser.add_argument("--cli", type=Path, required=True)
    args = parser.parse_args()
    app = args.app.resolve()
    with zipfile.ZipFile(args.zip, "w", compression=zipfile.ZIP_DEFLATED, compresslevel=9) as archive:
        for path in [app, *sorted(app.rglob("*"))]:
            name = str(Path(app.name) / path.relative_to(app)).replace("\\", "/")
            info = zipfile.ZipInfo(name + ("/" if path.is_dir() and not path.is_symlink() else ""))
            info.create_system = 3
            info.compress_type = zipfile.ZIP_DEFLATED
            if path.is_symlink():
                assert path.resolve().is_relative_to(app), "App symlink points outside bundle"
                info.external_attr = (stat.S_IFLNK | 0o777) << 16
                archive.writestr(info, path.readlink().as_posix().encode())
            elif path.is_dir():
                info.external_attr = ((stat.S_IFDIR | 0o755) << 16) | 0x10
                archive.writestr(info, b"")
            else:
                mode = 0o755 if path.stat().st_mode & 0o111 else 0o644
                info.external_attr = (stat.S_IFREG | mode) << 16
                archive.writestr(info, path.read_bytes())
    files = [(app / "Contents/MacOS/mcplink", "mcplink", 0o755),
             (ROOT / "LICENSE", "LICENSE", 0o644),
             (ROOT / "THIRD_PARTY_NOTICES.txt", "THIRD_PARTY_NOTICES.txt", 0o644)]
    with args.cli.open("wb") as output:
        with gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                for path, name, mode in files:
                    info = tarfile.TarInfo(name)
                    info.size, info.mode = path.stat().st_size, mode
                    with path.open("rb") as content:
                        archive.addfile(info, content)
    print("Packaged app and CLI with normalized metadata and licenses")


if __name__ == "__main__":
    main()
