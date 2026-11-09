"""Build a pinned downstream Flyline release on a developer machine only."""

from __future__ import annotations

import argparse
import gzip
import hashlib
import json
import os
import shutil
import subprocess
import tarfile
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def run(*args: str) -> str:
    return subprocess.check_output(args, cwd=ROOT, text=True).strip()


def digest(path: Path) -> str:
    return hashlib.sha256(path.read_bytes()).hexdigest()


def build(args: argparse.Namespace) -> None:
    pins = json.loads((ROOT / "remote-ssh/build.json").read_text())
    head = run("git", "rev-parse", "HEAD")
    if head != args.expected_commit:
        raise ValueError("HEAD differs from --expected-commit")
    if run("git", "status", "--porcelain", "--untracked-files=normal"):
        raise ValueError("Build requires a clean source checkout")
    for ancestor in (pins["upstream_base"], pins["code_commit"]):
        subprocess.run(
            ["git", "merge-base", "--is-ancestor", ancestor, head], cwd=ROOT, check=True
        )
    if digest(ROOT / "Cargo.lock") != pins["cargo_lock_sha256"]:
        raise ValueError("Cargo.lock differs from the pinned base")
    if args.output.exists():
        raise ValueError(f"Refusing to overwrite {args.output}")
    cargo = args.cargo.resolve()
    rustc = cargo.with_name("rustc")
    rust_version = run(str(rustc), "--version")
    if not rust_version.startswith(f"rustc {pins['rust_version']} "):
        raise ValueError(f"Use pinned Rust {pins['rust_version']}")
    host = run(str(rustc), "-vV").split("host: ")[1].splitlines()[0]
    if host != args.target:
        raise ValueError("This release requires a native target")
    env = os.environ | {
        "PATH": f"{cargo.parent}{os.pathsep}{os.environ.get('PATH', '')}",
        "SOURCE_DATE_EPOCH": pins["source_date_epoch"],
    }
    for key in (
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "RUSTC",
        "RUSTC_WRAPPER",
        "RUSTC_WORKSPACE_WRAPPER",
        "CARGO_TARGET_DIR",
    ):
        if env.get(key):
            raise ValueError(f"Unset {key} before building")
    command = [str(cargo), "build", "--release", "--locked", "--target", args.target]
    if args.offline:
        command.append("--offline")
    clean_commands = [
        [str(cargo), "clean", "--package", "flyline"],
        [str(cargo), "clean", "--package", "flyline", "--target", args.target],
    ]
    for clean in clean_commands:
        subprocess.run(clean, cwd=ROOT, env=env, check=True)
    # Refresh Git/build-time metadata even with a reused Cargo target cache.
    (ROOT / "build.rs").touch()
    subprocess.run(command, cwd=ROOT, env=env, check=True)
    library = (
        "libflyline.dylib" if args.target.endswith("apple-darwin") else "libflyline.so"
    )
    args.output.mkdir(parents=True)
    shutil.copy2(
        ROOT / "target" / args.target / "release" / library, args.output / library
    )
    shutil.copy2(ROOT / "LICENSE-MIT", args.output / "LICENSE-MIT")
    provenance = pins | {
        "source_repository": "https://github.com/fmmaciej/flyline",
        "source_commit": head,
        "port_commit": head,
        "source_tree": run("git", "rev-parse", "HEAD^{tree}"),
        "rustc": rust_version,
        "cargo": run(str(cargo), "--version"),
        "target": args.target,
        "build_command": command,
        "clean_commands": clean_commands,
        "library_member": library,
        "library_sha256": digest(args.output / library),
        "qualification": "Requires native qualification for these exact artifact bytes",
    }
    (args.output / "provenance.json").write_text(
        json.dumps(provenance, indent=2) + "\n"
    )
    archive = args.output / f"flyline-{pins['identity']}-{args.target}.tar.gz"
    with (
        archive.open("wb") as raw,
        gzip.GzipFile(
            filename="", mode="wb", fileobj=raw, mtime=int(pins["source_date_epoch"])
        ) as compressed,
        tarfile.open(fileobj=compressed, mode="w") as package,
    ):
        for member in (library, "LICENSE-MIT", "provenance.json"):
            path = args.output / member
            info = tarfile.TarInfo(member)
            info.size = path.stat().st_size
            info.mtime = int(pins["source_date_epoch"])
            info.mode = 0o755 if member == library else 0o644
            with path.open("rb") as content:
                package.addfile(info, content)
    (args.output / "SHA256SUMS").write_text(f"{digest(archive)}  {archive.name}\n")
    print(f"Built {archive}; not installed or published")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", type=Path, required=True)
    parser.add_argument("--expected-commit", required=True)
    parser.add_argument(
        "--target",
        choices=("aarch64-apple-darwin", "x86_64-unknown-linux-gnu"),
        required=True,
    )
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--offline", action="store_true")
    args = parser.parse_args()
    try:
        build(args)
    except (ValueError, subprocess.CalledProcessError) as exc:
        parser.exit(1, f"{exc}\n")


if __name__ == "__main__":
    main()
