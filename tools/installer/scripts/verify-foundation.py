#!/usr/bin/env python3
"""Verify the product-local installation support boundary without fetching Git."""
from pathlib import Path
import json
import tomllib


def verify(root):
    root = Path(root).resolve(strict=True)
    cargo = tomllib.loads((root / 'Cargo.toml').read_text())
    dependency = cargo['dependencies']['xcsc']
    if dependency != {'package': 'xscs-install-support', 'path': 'support',
                      'version': '=1.0.0', 'features': ['offline-maintenance']}:
        raise ValueError('helper must use the product-local installation support')
    support = (root / dependency['path']).resolve(strict=True)
    if not support.is_relative_to(root):
        raise ValueError('installation support escapes its source snapshot')
    manifest = tomllib.loads((support / 'Cargo.toml').read_text())
    if manifest['package']['name'] != 'xscs-install-support':
        raise ValueError('installation support package identity differs')
    imported = json.loads((support / 'UPSTREAM.json').read_text())
    if imported['revision'] != '6fca4299f5c34a79db1c7bd4e889b418d2a58e2d' or not imported['files']:
        raise ValueError('installation support import attribution is missing')
    if not (support / 'LICENSE').read_bytes().strip():
        raise ValueError('installation support original license is missing')
    for package in tomllib.loads((root / 'Cargo.lock').read_text())['package']:
        if package.get('source', '').startswith('git+'):
            raise ValueError('installation helper cannot fetch external Git packages')
    print('Product-local installation support and locked dependencies verified')


if __name__ == '__main__':
    verify(Path(__file__).resolve().parents[1])
