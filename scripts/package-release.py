#!/usr/bin/env python3
"""Create a deterministic gzip-compressed tar archive for a release asset."""

from __future__ import annotations

import argparse
import gzip
import tarfile
from pathlib import Path


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--root-dir", required=True)
    args = parser.parse_args()

    root = Path.cwd()
    binary = args.binary.resolve()
    if not binary.is_file():
        parser.error(f"binary does not exist: {binary}")

    files = [
        (binary, "cnowledje"),
        (root / "LICENSE", "LICENSE"),
        (root / "README.md", "README.md"),
    ]
    for source, _ in files:
        if not source.is_file():
            parser.error(f"required archive file does not exist: {source}")

    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("wb") as output:
        # gzip mtime=0 removes filesystem timestamps from the outer archive.
        with gzip.GzipFile(fileobj=output, mode="wb", filename="", mtime=0) as compressed:
            with tarfile.open(fileobj=compressed, mode="w", format=tarfile.USTAR_FORMAT) as archive:
                for source, name in files:
                    info = archive.gettarinfo(str(source), arcname=f"{args.root_dir}/{name}")
                    info.uid = 0
                    info.gid = 0
                    info.uname = ""
                    info.gname = ""
                    info.mtime = 0
                    info.devmajor = 0
                    info.devminor = 0
                    info.pax_headers = {}
                    if source.is_file():
                        with source.open("rb") as contents:
                            archive.addfile(info, contents)
                    else:
                        archive.addfile(info)


if __name__ == "__main__":
    main()
