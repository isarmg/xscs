#!/usr/bin/env python3
"""Create the one strict manifest accepted by a bound xscs binary."""

from __future__ import annotations

import hashlib
import json
import os
import re
import stat
import subprocess
import sys
from pathlib import Path
from typing import NoReturn


APPLICATION = "xscs"
VERSION = "1.0.3"
TARGET = "x86_64-unknown-linux-gnu"
CONTRACT_FORMAT = "xscs-release-v1"
MANIFEST_FORMAT = "xscs-files-v1"
MANIFEST_NAME = "RELEASE-MANIFEST.json"
MAX_ENTRIES = 10_000
MAX_FILE_BYTES = 512 * 1024 * 1024
MAX_RELEASE_BYTES = 1024 * 1024 * 1024
IDENTITY_KEYS = {
    "manifest_format",
    "application",
    "version",
    "api_prefix",
    "schema_revision",
    "schema_sha256",
    "target",
    "source_revision",
    "web_assets_sha256",
}
PORTABLE_NAME = re.compile(r"^[A-Za-z0-9._-]+$")
FULL_REVISION = re.compile(r"^[0-9a-f]{40}$")
SHA256 = re.compile(r"^[0-9a-f]{64}$")


def fail(message: str) -> NoReturn:
    raise SystemExit(f"release manifest: {message}")


def portable_relative(path: Path, root: Path) -> str:
    relative = path.relative_to(root)
    if not relative.parts or any(
        part in {"", ".", ".."} or PORTABLE_NAME.fullmatch(part) is None
        for part in relative.parts
    ):
        fail(f"non-portable release path: {relative}")
    value = relative.as_posix()
    if len(value) > 1024:
        fail("release path exceeds 1024 characters")
    return value


def read_identity(binary: Path) -> tuple[dict[str, object], bytes]:
    result = subprocess.run(
        [os.fspath(binary), "identity"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0 or result.stderr:
        fail("release binary could not report a clean identity")
    if not result.stdout.endswith(b"\n") or b"\n" in result.stdout[:-1]:
        fail("release binary identity must be exactly one JSON line")
    encoded = result.stdout[:-1]
    try:
        identity = json.loads(encoded)
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        fail(f"release binary identity is not JSON: {error}")
    if not isinstance(identity, dict) or set(identity) != IDENTITY_KEYS:
        fail("release binary identity has the wrong field set")
    if (
        identity["manifest_format"] != CONTRACT_FORMAT
        or identity["application"] != APPLICATION
        or identity["version"] != VERSION
        or identity["api_prefix"] != "/api/v1"
        or identity["schema_revision"] != 1
        or identity["target"] != TARGET
        or not isinstance(identity["schema_sha256"], str)
        or SHA256.fullmatch(identity["schema_sha256"]) is None
        or not isinstance(identity["web_assets_sha256"], str)
        or SHA256.fullmatch(identity["web_assets_sha256"]) is None
        or not isinstance(identity["source_revision"], str)
        or FULL_REVISION.fullmatch(identity["source_revision"]) is None
    ):
        fail("release binary is not the exact bound xscs 1.0.3 identity")
    return identity, encoded


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: write-release-manifest.py /absolute/releases/1.0.3")
    root = Path(sys.argv[1])
    if not root.is_absolute() or root.name != VERSION or root.parent.name != "releases":
        fail("root must be an absolute releases/1.0.3 directory")
    if root.resolve(strict=True) != root:
        fail("release root must not traverse symbolic links")
    root_stat = root.lstat()
    if not stat.S_ISDIR(root_stat.st_mode):
        fail("release root must be a real directory")

    manifest_path = root / MANIFEST_NAME
    if manifest_path.exists() or manifest_path.is_symlink():
        fail("release manifest already exists")
    binary = root / "bin/xscs"
    identity, encoded_identity = read_identity(binary)

    entries: list[dict[str, object]] = []
    total_bytes = 0
    for path in sorted(root.rglob("*"), key=lambda value: value.relative_to(root).as_posix()):
        relative = portable_relative(path, root)
        if relative == MANIFEST_NAME:
            fail("release manifest appeared during traversal")
        metadata = path.lstat()
        mode = stat.S_IMODE(metadata.st_mode)
        if mode & 0o222:
            fail(f"release entry is writable: {relative}")
        if stat.S_ISLNK(metadata.st_mode):
            fail(f"release contains a symbolic link: {relative}")
        if stat.S_ISDIR(metadata.st_mode):
            if mode != 0o555:
                fail(f"release directory must have mode 0555: {relative}")
            entries.append({"kind": "directory", "path": relative, "mode": "0555"})
        elif stat.S_ISREG(metadata.st_mode):
            if metadata.st_nlink != 1:
                fail(f"release file must have exactly one hard link: {relative}")
            if metadata.st_size > MAX_FILE_BYTES:
                fail(f"release file exceeds size limit: {relative}")
            total_bytes += metadata.st_size
            if total_bytes > MAX_RELEASE_BYTES:
                fail("release exceeds total size limit")
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            entries.append(
                {
                    "kind": "file",
                    "path": relative,
                    "mode": f"{mode:04o}",
                    "size": metadata.st_size,
                    "sha256": digest,
                }
            )
        else:
            fail(f"release contains a special file: {relative}")
        if len(entries) > MAX_ENTRIES:
            fail("release contains too many entries")

    manifest = {
        "manifest_format": MANIFEST_FORMAT,
        "application": APPLICATION,
        "version": VERSION,
        "source_revision": identity["source_revision"],
        "binary_identity_sha256": hashlib.sha256(encoded_identity).hexdigest(),
        "entries": entries,
    }
    encoded = json.dumps(
        manifest, ensure_ascii=False, separators=(",", ":"), sort_keys=True
    ).encode("utf-8") + b"\n"
    descriptor = os.open(
        manifest_path,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
        0o444,
    )
    try:
        with os.fdopen(descriptor, "wb", closefd=True) as output:
            output.write(encoded)
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        manifest_path.unlink(missing_ok=True)
        raise
    directory = os.open(root, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0))
    try:
        os.fsync(directory)
    finally:
        os.close(directory)


if __name__ == "__main__":
    main()
