#!/usr/bin/env python3
"""Build and publish one immutable, source-bound xscs 1.0.2 archive."""

from __future__ import annotations

import errno
import hashlib
import json
import os
import platform
import re
import shutil
import socket
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import NoReturn


APPLICATION = "xscs"
VERSION = "1.0.2"
TARGET = "x86_64-unknown-linux-gnu"
TAG = f"v{VERSION}"


def fail(message: str) -> NoReturn:
    raise SystemExit(f"package release: {message}")


def require_release_host() -> None:
    """Require the native platform used by the release verification smoke test."""
    try:
        gnu_libc = os.confstr("CS_GNU_LIBC_VERSION")
    except (AttributeError, OSError, ValueError):
        gnu_libc = None
    if (
        platform.system() != "Linux"
        or platform.machine() != "x86_64"
        or gnu_libc is None
        or not gnu_libc.startswith("glibc ")
    ):
        fail(
            "official Server releases require an x86_64 GNU/Linux build host; "
            f"detected system={platform.system()!r}, machine={platform.machine()!r}, "
            f"libc={gnu_libc!r}"
        )


def run(
    arguments: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    capture: bool = False,
) -> subprocess.CompletedProcess[bytes]:
    result = subprocess.run(
        arguments,
        cwd=cwd,
        env=env,
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.PIPE if capture else None,
        check=False,
    )
    if result.returncode != 0:
        if capture:
            sys.stderr.buffer.write(result.stdout)
            sys.stderr.buffer.write(result.stderr)
        fail(f"command failed ({result.returncode}): {' '.join(arguments)}")
    return result


def git_output(source: Path, *arguments: str) -> str:
    result = run(["git", *arguments], cwd=source, capture=True)
    if result.stderr:
        sys.stderr.buffer.write(result.stderr)
    try:
        return result.stdout.decode("utf-8").strip()
    except UnicodeDecodeError as error:
        fail(f"Git output is not UTF-8: {error}")


def require_clean_source(source: Path) -> None:
    status = git_output(source, "status", "--porcelain=v1", "--untracked-files=all")
    if status:
        fail("source tree must be completely clean before and after the build")


def chmod_tree_for_cleanup(root: Path) -> None:
    if not root.exists() or root.is_symlink():
        return
    def chmod_physical(path: Path, mode: int) -> None:
        try:
            descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
        except OSError as error:
            if error.errno in (errno.ENOENT, errno.ELOOP):
                return
            raise
        try:
            metadata = os.fstat(descriptor)
            if stat.S_ISREG(metadata.st_mode) or stat.S_ISDIR(metadata.st_mode):
                os.fchmod(descriptor, mode)
        finally:
            os.close(descriptor)
    for directory, child_directories, files in os.walk(root, topdown=False):
        for name in files:
            chmod_physical(Path(directory) / name, 0o600)
        for name in child_directories:
            chmod_physical(Path(directory) / name, 0o700)
    chmod_physical(root, 0o700)


def copy_exclusive(source: Path, destination: Path) -> None:
    descriptor = os.open(
        destination,
        os.O_WRONLY | os.O_CREAT | os.O_EXCL | getattr(os, "O_NOFOLLOW", 0),
        0o444,
    )
    try:
        with source.open("rb") as input_file, os.fdopen(
            descriptor, "wb", closefd=True
        ) as output_file:
            shutil.copyfileobj(input_file, output_file, length=1024 * 1024)
            output_file.flush()
            os.fsync(output_file.fileno())
    except BaseException:
        destination.unlink(missing_ok=True)
        raise


def relocated_smoke(extracted: Path, temporary: Path) -> None:
    state = temporary / "smoke-state/db"
    state.mkdir(parents=True, mode=0o700)
    state.chmod(0o700)
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as listener:
        listener.bind(("127.0.0.1", 0))
        port = listener.getsockname()[1]
    environment = os.environ.copy()
    environment.update(
        {
            "XSCS_DATABASE_URL": f"sqlite://{state / 'app.db'}",
            "XSCS_CREDENTIAL_KEY": "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=",
            "XSCS_BIND": f"127.0.0.1:{port}",
            "XSCS_PRODUCTION": "false",
            "XSCS_BOOTSTRAP_ADMIN_USERNAME": "release-smoke",
            "XSCS_BOOTSTRAP_ADMIN_PASSWORD": "release-smoke-password",
        }
    )
    subprocess.run([os.fspath(extracted / "bin/xscs"), "init", "--json"],
        cwd=Path("/"), env=environment, stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE, check=True)
    installation = extracted.parent.parent
    installation.chmod(0o755)
    extracted.parent.chmod(0o755)
    pointer = installation / "current"
    pointer.symlink_to(extracted)
    log_path = temporary / "relocated-smoke.log"
    with log_path.open("wb") as log:
        process = subprocess.Popen(
            [
                os.fspath(extracted / "bin/xscs"),
                "run",
                "--release-root",
                os.fspath(pointer),
            ],
            cwd=Path("/"),
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
        )
        try:
            ready = False
            for _ in range(120):
                if process.poll() is not None:
                    break
                try:
                    with urllib.request.urlopen(
                        f"http://127.0.0.1:{port}/readyz", timeout=1
                    ) as response:
                        ready = (response.status == 200 and response.getheader("x-service") == "xscs" and response.read(128) == b'{"ready":true}')
                except (urllib.error.URLError, TimeoutError):
                    pass
                if ready:
                    break
                time.sleep(0.1)
            if not ready:
                log.flush()
                sys.stderr.buffer.write(log_path.read_bytes())
                fail("relocated release did not become live")
            inventory = json.loads((extracted / "web-assets.json").read_bytes())
            for asset in inventory["files"]:
                with urllib.request.urlopen(f"http://127.0.0.1:{port}/{asset['path']}", timeout=2) as response:
                    body = response.read()
                    if (response.status != 200 or len(body) != asset["size"]
                        or hashlib.sha256(body).hexdigest() != asset["sha256"]
                        or response.headers.get("Content-Type") != asset["content_type"]
                        or response.headers.get("X-Content-Type-Options") != "nosniff"):
                        fail(f"relocated embedded asset does not match its compiled inventory: {asset['path']}")
            with urllib.request.urlopen(f"http://127.0.0.1:{port}/", timeout=2) as response:
                if "no-store" not in {value.strip().lower() for value in response.headers.get("Cache-Control", "").split(",")}:
                    fail("relocated browser entry must not be cached")
        finally:
            if process.poll() is None:
                process.terminate()
                try:
                    process.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    process.kill()
                    process.wait(timeout=5)

    asset_path = extracted / "web-assets.json"
    asset_path.chmod(0o644)
    with asset_path.open("ab") as asset:
        asset.write(b"\ntampered\n")
    asset_path.chmod(0o444)
    rejection = subprocess.run(
        [
            os.fspath(extracted / "bin/xscs"),
            "verify-release",
            "--root",
            os.fspath(extracted),
        ],
        cwd=Path("/"),
        stdin=subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if rejection.returncode == 0 or not any(
        marker in rejection.stderr for marker in [b"digest mismatch", b"size mismatch"]
    ):
        fail("release verifier did not reject a tampered embedded Web inventory")


def main() -> None:
    if len(sys.argv) != 2:
        fail("usage: package-release.py /absolute/output-directory")
    require_release_host()
    source = Path(__file__).resolve(strict=True).parent.parent
    output_directory = Path(sys.argv[1])
    if not output_directory.is_absolute():
        fail("output directory must be absolute")
    if output_directory.resolve(strict=True) != output_directory:
        fail("output directory must be an existing real directory")
    if source == output_directory or source in output_directory.parents:
        fail("output directory must be outside the source tree")

    for command in ["cargo", "git", "npm", "python3", "tar"]:
        if shutil.which(command) is None:
            fail(f"required command is missing: {command}")
    require_clean_source(source)
    revision = git_output(source, "rev-parse", "--verify", "HEAD")
    if len(revision) != 40 or any(character not in "0123456789abcdef" for character in revision):
        fail("HEAD is not a full lowercase 40-hex Git commit")
    if git_output(source, "cat-file", "-t", f"refs/tags/{TAG}") != "tag":
        fail(f"official packaging requires the annotated tag {TAG}")
    if git_output(source, "rev-parse", f"refs/tags/{TAG}^{{commit}}") != revision:
        fail(f"annotated tag {TAG} does not identify HEAD")
    source_epoch_text = git_output(source, "show", "-s", "--format=%ct", revision)
    if not source_epoch_text.isdigit():
        fail("source commit timestamp is invalid")

    archive_name = f"{APPLICATION}-{VERSION}-{TARGET}.tar.gz"
    archive_output = output_directory / archive_name
    checksum_output = output_directory / f"{archive_name}.sha256"
    if archive_output.exists() or archive_output.is_symlink():
        fail(f"refusing to replace existing archive: {archive_output}")
    if checksum_output.exists() or checksum_output.is_symlink():
        fail(f"refusing to replace existing checksum: {checksum_output}")

    temporary = Path(tempfile.mkdtemp(prefix="xscs-release-"))
    published_archive = False
    try:
        releases = temporary / "releases"
        root = releases / VERSION
        web_stage = temporary / "web"
        for directory in [root / "bin", root / "systemd", web_stage]:
            directory.mkdir(parents=True, exist_ok=False)

        run(["npm", "ci"], cwd=source / "web")
        builder = source / "web/node_modules/.bin/xcss-build-server"
        build_config = source / "xcss-web-build.json"
        run([os.fspath(builder), "--config", os.fspath(build_config), "--mode", "release", "--web-only", "--no-install", "--dist", os.fspath(web_stage)], cwd=source)

        cargo_target = Path(
            os.environ.get("SUNSHINE_RELEASE_CARGO_TARGET_DIR", source / "target")
        )
        if not cargo_target.is_absolute():
            fail("SUNSHINE_RELEASE_CARGO_TARGET_DIR must be absolute")
        build_environment = os.environ.copy()
        build_environment.update(
            {
                "CARGO_TARGET_DIR": os.fspath(cargo_target),
                "CARGO_INCREMENTAL": "0",
                "XSCS_SOURCE_REVISION": revision,
            }
        )
        run([os.fspath(builder), "--config", os.fspath(build_config), "--mode", "release", "--rust-only", "--no-install", "--dist", os.fspath(web_stage), "--source-revision", revision], cwd=source, env=build_environment)
        built_binary = cargo_target / TARGET / "release/xscs"
        if not built_binary.is_file() or built_binary.is_symlink():
            fail("Cargo did not produce the expected release binary")

        shutil.copyfile(built_binary, root / "bin/xscs")
        shutil.copyfile(
            source / "deploy/xscs.service",
            root / "systemd/xscs.service",
        )
        shutil.copyfile(source / "docs/operations.md", root / "README.md")
        inventory = run([os.fspath(built_binary), "web-assets"], cwd=source, capture=True).stdout
        if not inventory.endswith(b"\n"):
            fail("compiled Web inventory must be one newline-terminated JSON document")
        (root / "web-assets.json").write_bytes(inventory[:-1])

        executable_paths = [root / "bin/xscs"]
        for path in executable_paths:
            path.chmod(0o555)
        for path in root.rglob("*"):
            if path.is_symlink():
                fail(f"release staging contains a symbolic link: {path}")
            if path.is_dir():
                path.chmod(0o555)
            elif path not in executable_paths:
                path.chmod(0o444)

        run(
            ["python3", os.fspath(source / "scripts/write-release-manifest.py"), os.fspath(root)],
            cwd=source,
        )
        root.chmod(0o555)
        run(
            [
                os.fspath(root / "bin/xscs"),
                "verify-release",
                "--root",
                os.fspath(root),
            ],
            cwd=Path("/"),
        )
        require_clean_source(source)

        archive = temporary / archive_name
        archive_members = temporary / "archive-members"
        files = []
        directories = [Path(VERSION)]
        for path in root.rglob("*"):
            member = Path(VERSION) / path.relative_to(root)
            if path.is_dir():
                directories.append(member)
            else:
                files.append(member)
        files.sort(key=lambda path: path.as_posix())
        directories.sort(key=lambda path: (-len(path.parts), path.as_posix()))
        archive_members.write_bytes(
            b"\0".join(os.fspath(path).encode("ascii") for path in [*files, *directories])
            + b"\0"
        )
        run(
            [
                "tar",
                "--create",
                "--gzip",
                "--file",
                os.fspath(archive),
                "--directory",
                os.fspath(releases),
                "--sort=name",
                f"--mtime=@{source_epoch_text}",
                "--owner=0",
                "--group=0",
                "--numeric-owner",
                "--format=posix",
                "--pax-option=delete=atime,delete=ctime",
                "--no-recursion",
                "--null",
                "--files-from",
                os.fspath(archive_members),
            ],
            cwd=source,
        )

        extracted_releases = temporary / "extracted/opt/isarmg/xscs/releases"
        extracted_releases.mkdir(parents=True)
        run(
            [
                "tar",
                "--extract",
                "--gzip",
                "--file",
                os.fspath(archive),
                "--directory",
                os.fspath(extracted_releases),
                "--no-same-owner",
                "--same-permissions",
                "--delay-directory-restore",
            ],
            cwd=source,
        )
        extracted = extracted_releases / VERSION
        if sorted(path.name for path in extracted_releases.iterdir()) != [VERSION]:
            fail("release archive has an unexpected top-level layout")
        run(
            [
                os.fspath(extracted / "bin/xscs"),
                "verify-release",
                "--root",
                os.fspath(extracted),
            ],
            cwd=Path("/"),
        )
        relocated_smoke(extracted, temporary)

        digest = hashlib.sha256(archive.read_bytes()).hexdigest()
        checksum = temporary / f"{archive_name}.sha256"
        checksum.write_text(f"{digest}  {archive_name}\n", encoding="ascii")
        checksum.chmod(0o444)
        copy_exclusive(archive, archive_output)
        published_archive = True
        try:
            copy_exclusive(checksum, checksum_output)
        except BaseException:
            archive_output.unlink(missing_ok=True)
            published_archive = False
            raise
        directory_descriptor = os.open(
            output_directory, os.O_RDONLY | getattr(os, "O_DIRECTORY", 0)
        )
        try:
            os.fsync(directory_descriptor)
        finally:
            os.close(directory_descriptor)
        print(archive_output)
        print(checksum_output)
    finally:
        if published_archive and not checksum_output.exists():
            archive_output.unlink(missing_ok=True)
        chmod_tree_for_cleanup(temporary)
        shutil.rmtree(temporary)


if __name__ == "__main__":
    main()
