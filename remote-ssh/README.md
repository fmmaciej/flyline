# Downstream Vim release for remote-ssh

This branch ports upstream Vim PR #974 onto Flyline 1.10.0 and carries bounded
editing/history/unload fixes. It is an experimental downstream build, not an
official Flyline release or complete Vim emulation. MIT notices are retained.

`master` remains the upstream tracking branch. `remote-ssh-vim` is based on the
pinned stable commit in `build.json`, followed by six separately recorded patch
commits. Each records its original patch digest. The original upstream history
is preserved. Keep code changes small and move to an official release once its
Vim support and fixes pass the integration matrix. Do not rewrite release tags.

Use downstream tags such as `remote-ssh-v1.10.0-pr974.aae68bb4.r5`. They do not
match the upstream `v*.*.*` release workflow, which builds a different asset
matrix and expects Cargo.toml's unmodified package version. Do not run that
workflow to publish these custom packages. Downstream release assets are built
and qualified explicitly; no Linux or Intel macOS support is implied.

## Developer build

Use native macOS arm64, a C linker/SDK, Python 3.13+, and Rust/Cargo 1.99.1.
Remote runtime hosts must never compile this source or run this builder.
From a clean checkout of the exact release commit:

```bash
python3 remote-ssh/build.py \
  --expected-commit FULL_RELEASE_COMMIT \
  --cargo /path/to/rust-1.99.1/bin/cargo \
  --target aarch64-apple-darwin \
  --output /tmp/flyline-artifact
```

Supply the real Cargo binary, not a rustup shim. Use a fresh output directory.
The builder verifies HEAD, ancestry, the unchanged Cargo.lock, native target and
pinned Rust, uses locked dependencies, refreshes Git build metadata, and writes
the library, MIT license, provenance, tar.gz and SHA256SUMS. Add `--offline` once
the locked dependencies are cached. No moving dependency update is allowed.
Different SDKs/linkers/paths may change native bytes; qualification follows the
actual library/archive hash, not the version name alone.

Before pushing, run the quality checks required by AGENTS.md: rustfmt,
workspace library tests, Clippy with warnings denied, and rustdoc. Avoid the
unrestricted Docker integration suite. Native editor/management/history/
completion/terminal/performance tests live in the remote-ssh integration project.
Run them against the final built artifact before publishing supported assets.

## Distribution

Publish a prerelease on `fmmaciej/flyline` whose tag points at the exact source
commit in provenance. GitHub's source archives then refer to this source tree.
Attach the qualified runtime tar.gz, SHA256SUMS, provenance, MIT license and
qualification record. Preserve the distinction between declared asset support
and the tested Bash ABI. Initial qualification is local macOS arm64 only.

The remote-ssh project pins the exact release asset/checksum and supplies the
optional installer/session integration. It must not discover moving releases,
build on remote hosts, replace Atuin storage, or enable the editor by default.
