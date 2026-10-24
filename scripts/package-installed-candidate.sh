#!/usr/bin/env bash
# Build the candidate with installation tools committed in this product tree.
set -euo pipefail
[[ $# == 4 ]] || { echo 'Usage: package-installed-candidate.sh BINARY METADATA INSTALLER OUTPUT' >&2; exit 2; }
source_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
for value in "$@"; do [[ "$value" == /* ]] || { echo 'Candidate paths must be absolute.' >&2; exit 2; }; done
binary="$1"; metadata="$2"; installer="$3"; output="$4"
[[ "$(cd -- "$installer" && pwd -P)" == "$source_root/tools/installer" ]]
export GIT_NO_REPLACE_OBJECTS=1
[[ -z "$(git -C "$source_root" status --porcelain=v1 --untracked-files=all)" ]]
python3 -I -B "$installer/scripts/verify-foundation.py"
exec python3 -I -B "$installer/scripts/package-installed-candidate.py" xscs "$source_root" "$binary" "$metadata" "$output"
