#!/usr/bin/env python3
"""Validate that a release tag matches the repository's version metadata."""

from __future__ import annotations

import json
import re
import subprocess
import sys
from pathlib import Path


ROOT = Path(__file__).resolve().parents[1]
SEMVER_IDENTIFIER = re.compile(r"^[0-9A-Za-z-]+$")


def fail(message: str) -> "NoReturn":
    print(f"release version check failed: {message}", file=sys.stderr)
    raise SystemExit(1)


def validate_semver(value: str) -> None:
    if value.startswith("v"):
        value = value[1:]
    else:
        fail("tag must start with 'v'")

    core_and_pre, _, build = value.partition("+")
    core, separator, prerelease = core_and_pre.partition("-")
    core_parts = core.split(".")
    if len(core_parts) != 3 or any(
        not part.isdigit() or (len(part) > 1 and part.startswith("0"))
        for part in core_parts
    ):
        fail("tag must use SemVer MAJOR.MINOR.PATCH")

    if separator:
        identifiers = prerelease.split(".")
        if not identifiers or any(
            not SEMVER_IDENTIFIER.fullmatch(identifier)
            or (identifier.isdigit() and len(identifier) > 1 and identifier.startswith("0"))
            for identifier in identifiers
        ):
            fail("tag contains an invalid SemVer prerelease")

    if build:
        identifiers = build.split(".")
        if not identifiers or any(not SEMVER_IDENTIFIER.fullmatch(identifier) for identifier in identifiers):
            fail("tag contains an invalid SemVer build metadata suffix")


def cargo_version() -> str:
    output = subprocess.check_output(
        [
            "cargo",
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--locked",
        ],
        cwd=ROOT,
        text=True,
    )
    metadata = json.loads(output)
    packages = [package for package in metadata["packages"] if package["name"] == "cnowledje"]
    if len(packages) != 1:
        fail("expected exactly one cnowledje package in cargo metadata")
    return packages[0]["version"]


def apm_version() -> str:
    text = (ROOT / "apm.yml").read_text(encoding="utf-8")
    match = re.search(r"(?m)^version:\s*[\"']?([^\s\"'#]+)[\"']?\s*$", text)
    if match is None:
        fail("apm.yml has no top-level version")
    return match.group(1)


def main() -> None:
    if len(sys.argv) > 2:
        fail("usage: check-release-version.py [vX.Y.Z[-prerelease][+build]]")

    cargo = cargo_version()
    apm = apm_version()
    if apm != cargo:
        fail(f"apm.yml version {apm} does not match Cargo.toml version {cargo}")

    if len(sys.argv) == 1:
        print(cargo)
        return

    tag = sys.argv[1]
    validate_semver(tag)
    expected = tag[1:]
    if cargo != expected:
        fail(f"tag {tag} does not match Cargo.toml version {cargo}")

    print(cargo)


if __name__ == "__main__":
    main()
