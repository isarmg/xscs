"""Read actual build inputs from the containing immutable product Git commit."""
import os
from pathlib import Path
import re
import subprocess

import release_sbom as sbom
from release_manifest import bounded


def git(root, *arguments):
    environment = dict(os.environ, GIT_NO_REPLACE_OBJECTS='1', GIT_TERMINAL_PROMPT='0')
    try:
        return subprocess.check_output(['git', '-C', str(root), *arguments],
                                       env=environment, stderr=subprocess.DEVNULL, timeout=30)
    except (subprocess.CalledProcessError, subprocess.TimeoutExpired) as error:
        raise ValueError('product source is not available in the declared Git commit') from error


def identity(root):
    root = sbom.physical_directory(root).resolve(strict=True)
    if Path(git(root, 'rev-parse', '--show-toplevel').decode().strip()).resolve() != root:
        raise ValueError('runtime sources must belong to the product repository root')
    revision = git(root, 'rev-parse', 'HEAD').decode().strip()
    if not re.fullmatch('[0-9a-f]{40}', revision):
        raise ValueError('runtime sources require a complete product commit')
    return root, revision


def committed_file(root, revision, relative):
    relative = sbom.relative(relative)
    path = root / relative
    sbom.physical_directory(path.parent)
    current = bounded(path, 256 * 1024 * 1024)
    original = git(root, 'show', revision + ':' + relative)
    if current != original:
        raise ValueError('runtime input differs from its original product source revision')
    return current
