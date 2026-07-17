# Releasing cnowledje

This document describes the maintainer workflow for publishing a cnowledje
release on GitHub.

## Version policy

The canonical version source is `[package].version` in `Cargo.toml`.

- Release tags use SemVer with a `v` prefix: `vMAJOR.MINOR.PATCH`.
- SemVer prerelease and build metadata suffixes are accepted, for example
  `v0.2.0-rc.1` and `v0.2.0+build.1`.
- `flake.nix` reads the version directly from `Cargo.toml`.
- `apm.yml` repeats the version because its YAML format cannot import Cargo
  metadata. CI rejects drift between the two files.
- The release workflow never rewrites `Cargo.toml` from an environment variable
  or tag. The tag must already match the committed source.

For a normal version bump, update `Cargo.toml` and `apm.yml` in the same
commit. When the Cargo package version changes, run Cargo without `--locked`
once so the root package version in `Cargo.lock` is updated, then use locked
commands for all verification and release builds.

## Prerequisites

Before the first release:

1. Configure GitHub CLI authentication with permission to push tags and create
   releases:

   ```bash
   gh auth login -h github.com
   gh auth status
   ```

2. Enable the repository's GitHub **immutable releases** setting.
3. Restrict creation, update, and deletion of `v*` tags with a repository ruleset
   if the repository's governance requires it.
4. Ensure the GitHub Actions workflow has permission to write repository contents.
   Only the final `publish` job requests `contents: write`.

Immutable releases are enforced when the release is published. Draft releases
remain mutable so that all assets can be assembled before publication.

## Pre-release checks

From the repository root:

```bash
VERSION=0.1.0

# Update Cargo.toml: version = "${VERSION}"
# Update apm.yml:    version: ${VERSION}

# Update Cargo.lock's root package version.
cargo check

# Confirm Cargo.toml and apm.yml agree.
python3 scripts/check-release-version.py

# Run the same quality gates used by CI.
cargo fmt --all -- --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --all-targets --locked
nix flake check --no-build
```

For the initial release, keep the current version (`0.1.0`) and use
`v0.1.0`. Do not bump the version only to create the first release.

## Commit and tag

Commit the version and lockfile changes first. Do not create the release tag
until the commit is on the release branch and the normal CI checks are green.

```bash
git add Cargo.toml Cargo.lock apm.yml
git commit -m "Release v${VERSION}"
git push origin main
git tag -a "v${VERSION}" -m "Release v${VERSION}"
git push origin "v${VERSION}"
```

For the initial release:

```bash
git tag -a v0.1.0 -m "Release v0.1.0"
git push origin v0.1.0
```

The release workflow is triggered by the `v*` tag push. It validates the full
SemVer syntax and requires the tag version to match both `Cargo.toml` and
`apm.yml`. Invalid or mismatched tags fail before any release is created.

## Release workflow

`.github/workflows/release.yml` performs the following steps:

1. Validate the tag and package versions.
2. Run formatting, clippy, and tests on Ubuntu.
3. Build native Apple Silicon (`aarch64-apple-darwin`) and Intel
   (`x86_64-apple-darwin`) binaries on `macos-15` and `macos-15-intel`.
4. Package each binary with `LICENSE` and `README.md` into a deterministic
   tarball.
5. Download both tarballs in one publish job and generate `SHA256SUMS`.
6. Verify every tarball against `SHA256SUMS`.
7. Create or reuse a GitHub draft release.
8. Upload both tarballs and `SHA256SUMS` to the draft.
9. Publish the complete draft release once all assets are present.

The archive names are:

```text
cnowledje-v${VERSION}-aarch64-apple-darwin.tar.gz
cnowledje-v${VERSION}-x86_64-apple-darwin.tar.gz
SHA256SUMS
```

If a workflow run fails after creating a draft or uploading some assets, rerun
the workflow. An existing draft is reused and same-named draft assets are
replaced. If a release is already published, the workflow refuses to modify
it. Fixes to a published immutable release require a new patch version and a
new tag.

## Post-release verification

After the workflow succeeds, inspect the release and checksum manifest:

```bash
gh release view "v${VERSION}" \
  --repo turtton/cnowledje \
  --json isImmutable,assets

gh release download "v${VERSION}" \
  --repo turtton/cnowledje \
  --pattern 'SHA256SUMS' \
  --pattern '*.tar.gz'

shasum -a 256 -c SHA256SUMS
```

Verify both architectures from clean macOS environments when possible:

```bash
# Extract the architecture-specific archive, then enter its root directory:
TARGET=aarch64-apple-darwin # or x86_64-apple-darwin
cd "cnowledje-v${VERSION}-${TARGET}"
./cnowledje --version
```

The output must be `cnowledje ${VERSION}`.

Nix and Cargo consumers can verify the same tag independently:

```bash
nix profile install "github:turtton/cnowledje/v${VERSION}"
cargo install --git https://github.com/turtton/cnowledje \
  --tag "v${VERSION}" --locked
```

## Current distribution limitation

The GitHub workflow currently publishes unsigned macOS archives. Add
Developer ID signing and Apple notarization before treating the archives as a
public, non-interactive macOS installation path.
