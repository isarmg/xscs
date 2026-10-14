#!/usr/bin/env bash
# Use the reviewed XSSI producer with the exact release binary and build receipts.
# This CI candidate is separate from the annotated-tag-only formal release path.
set -euo pipefail
[[ $# == 4 ]] || { echo 'Usage: package-installed-candidate.sh BINARY METADATA XSSI OUTPUT' >&2; exit 2; }
source_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
for value in "$@"; do [[ "$value" == /* ]] || { echo 'Candidate paths must be absolute.' >&2; exit 2; }; done
binary="$1"; metadata="$2"; installer="$3"; output="$4"
# Check the executable producer before invoking code from the supplied checkout.
revision="$(python3 -I -B - "$source_root/config/xssi.lock" <<'PY'
import pathlib,re,sys
raw=pathlib.Path(sys.argv[1]).read_text(encoding='ascii')
match=re.fullmatch(r'git_rev=([0-9a-f]{40})\n',raw)
if match is None or match[1]=='0'*40:raise SystemExit('Invalid immutable XSSI pin')
print(match[1])
PY
)"
export GIT_NO_REPLACE_OBJECTS=1
[[ "$(git -C "$installer" rev-parse HEAD)" == "$revision" ]]
[[ -z "$(git -C "$installer" status --porcelain=v1 --untracked-files=all)" ]]
exec python3 -I -B "$installer/scripts/package-installed-candidate.py" xscs "$source_root" "$binary" "$metadata" "$output"
