#!/usr/bin/env bash
# Ordinary server installation contract. No daemon is started by setup.
set -euo pipefail
readonly app=xscs
readonly account="isarmg-$app"
readonly unit="$app.service"
readonly source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
mode="${1:-}"
case "$mode" in
  setup) [[ $# -ge 2 && $# -le 3 ]] || { echo "usage: $0 setup CONFIG_JSON [USERNAME] | upgrade" >&2; exit 2; }; chosen_config="$2"; username="${3:-admin}" ;;
  upgrade) [[ $# == 1 ]] || { echo "usage: $0 setup CONFIG_JSON [USERNAME] | upgrade" >&2; exit 2; } ;;
  *) echo "usage: $0 setup CONFIG_JSON [USERNAME] | upgrade" >&2; exit 2 ;;
esac
# Test-only fixture root; never permits real account or service changes.
root="${XSCS_INSTALL_ROOT:-/}"
test_mode="${XSCS_INSTALL_TEST:-0}"
fail() { printf '%s install: %s\n' "$app" "$*" >&2; exit 1; }
[[ "$test_mode" == 0 || "$test_mode" == 1 ]] || fail 'invalid fixture mode'
if [[ "$test_mode" == 1 ]]; then
  [[ "$root" = /* && "$root" != / && -d "$root" && ! -L "$root" ]] || fail 'fixture requires a private alternate root'
else
  [[ "$root" == / && "$EUID" == 0 ]] || fail 'run as root'
fi
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || fail 'Linux x86_64 is required'
root="$(cd -- "$root" && pwd -P)"
[[ "$test_mode" != 1 || "$root" != / ]] || fail 'fixture refuses the canonical filesystem root'
rooted() { printf '%s%s\n' "${root%/}" "$1"; }
regular() { [[ -f "$1" && ! -L "$1" && "$(stat -c %h -- "$1")" == 1 ]] || fail "expected single-link regular file: $1"; }
check_chain() {
  local path="$1" current='' component
  local -a parts
  IFS=/ read -r -a parts <<<"${path#/}"
  for component in "${parts[@]}"; do
    [[ "$component" != . && "$component" != .. ]] || fail 'path traversal is forbidden'
    current="$current/$component"
    if [[ -e "$(rooted "$current")" || -L "$(rooted "$current")" ]]; then
      [[ -d "$(rooted "$current")" && ! -L "$(rooted "$current")" ]] || fail "unsafe directory: $current"
    fi
  done
}
readonly config_dir="$(rooted "/etc/isarmg/$app")"
readonly config="$config_dir/config.json"
readonly binary="$(rooted "/opt/isarmg/$app/bin/$app")"
readonly installed_unit="$(rooted "/etc/systemd/system/$unit")"
for path in "/opt/isarmg/$app/bin" /etc/systemd/system "/etc/isarmg/$app"; do check_chain "$path"; done
regular "$source_dir/bin/$app"
regular "$source_dir/systemd/$unit"
[[ -x "$source_dir/bin/$app" ]] || fail 'packaged binary is not executable'
for path in "$binary" "$installed_unit"; do
  if [[ -e "$path" || -L "$path" ]]; then regular "$path"; fi
done
run_service() {
  if [[ "$test_mode" == 1 ]]; then env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin XSCS_INSTALL_ROOT="$root" XSCS_INSTALL_TEST=1 "$binary" --config "$config" --data-dir "$(rooted "$data_dir")" "$@";
  else runuser -u "$account" -- env -i PATH=/usr/sbin:/usr/bin:/sbin:/bin "$binary" --config "$config" "$@"; fi
}
service() {
  if [[ "$test_mode" == 1 ]]; then
    printf '%s\n' "$*" >> "$root/service-actions"
    if [[ "$1" == is-active ]]; then [[ -f "$root/service-active" ]]; else return 0; fi
  else systemctl "$@"; fi
}
selected_config="$config"
if [[ "$mode" == setup ]]; then selected_config="$chosen_config"; fi
regular "$selected_config"
data_dir="$(python3 - "$selected_config" "$app" <<'PY'
import json, pathlib, sys
with open(sys.argv[1], encoding='utf-8') as stream: value = json.load(stream)
path = value.get('data_dir', '/var/lib/isarmg/' + sys.argv[2])
if not isinstance(path, str) or not path.startswith('/') or path == '/' or any(ord(c) < 32 or ord(c) == 127 for c in path) or any(part in {'.', '..'} for part in path.split('/')):
    raise SystemExit('data_dir must be an absolute non-root directory without traversal')
candidate = pathlib.PurePosixPath('/' + path.lstrip('/'))
for fixed in ['/opt/isarmg/' + sys.argv[2], '/etc/isarmg/' + sys.argv[2], '/etc/systemd/system', '/run/isarmg/' + sys.argv[2]]:
    boundary = pathlib.PurePosixPath(fixed)
    if candidate == boundary or candidate in boundary.parents or boundary in candidate.parents:
        raise SystemExit('data_dir must not overlap program, configuration, service or runtime directories')
print(str(candidate))
PY
)"
check_chain "$data_dir"
write_state_access() {
  local directory="$(rooted "/etc/systemd/system/$unit.d")"
  check_chain "/etc/systemd/system/$unit.d"
  install -d -m 0755 -- "$directory"
  staging="$(mktemp -- "$directory/.state.XXXXXX")"
  python3 - "$data_dir" > "$staging" <<'PYCODE'
import sys
path = sys.argv[1].replace('\\', '\\\\').replace('"', '\\"').replace('%', '%%')
print('[Service]\nReadWritePaths=\nReadWritePaths="' + path + '"')
PYCODE
  chmod 0644 -- "$staging"
  [[ "$test_mode" == 1 ]] || chown root:root -- "$directory" "$staging"
  mv -fT -- "$staging" "$directory/state.conf"
  staging=''
}
staging=''
trap '[[ -z "$staging" ]] || rm -f -- "$staging"' EXIT
replace_file() {
  local source="$1" destination="$2" permissions="$3"
  staging="$(mktemp -- "$(dirname -- "$destination")/.$app-install.XXXXXX")"
  install -m "$permissions" -- "$source" "$staging"
  [[ "$test_mode" == 1 ]] || chown root:root -- "$staging"
  mv -fT -- "$staging" "$destination"
  staging=''
}
if [[ "$mode" == setup ]]; then
  [[ ! -e "$config" && ! -L "$config" ]] || fail 'configuration already exists; setup never replaces an installation'
  regular "$chosen_config"
  check_chain "$data_dir"
  state="$(rooted "$data_dir")"
  if [[ -d "$state" ]]; then
    [[ -z "$(find "$state" -mindepth 1 -maxdepth 1 -print -quit)" ]] || fail 'setup requires empty state'
  fi
  if [[ "$test_mode" == 0 ]]; then
    getent group "$account" >/dev/null || groupadd --system "$account"
    id -u "$account" >/dev/null 2>&1 || useradd --system --gid "$account" --home-dir "$data_dir" --no-create-home --shell /usr/sbin/nologin "$account"
    [[ "$(id -u "$account")" != 0 && "$(id -g "$account")" == "$(getent group "$account" | cut -d: -f3)" ]] || fail 'service account must be non-root with its same-named group'
  fi
  install -d -m 0755 -- "$(dirname -- "$binary")" "$(dirname -- "$installed_unit")"
  install -d -m 0700 -- "$config_dir"
  (umask 077; set -C; cat -- "$chosen_config" > "$config")
  chmod 0600 -- "$config"
  [[ "$test_mode" == 1 ]] || chown "$account:$account" -- "$config_dir" "$config"
  replace_file "$source_dir/bin/$app" "$binary" 0755
  replace_file "$source_dir/systemd/$unit" "$installed_unit" 0644
  run_service config validate
  install -d -m 0700 -- "$state"
  [[ "$test_mode" == 1 ]] || chown "$account:$account" -- "$state"
  # The caller's stdin is passed directly to the explicit initializer.
  run_service init --username "$username"
  run_service check
  write_state_access
  service daemon-reload
  printf 'Initialized %s. Start it with: sudo systemctl enable --now %s\n' "$app" "$unit"
else
  regular "$config"
  regular "$binary"
  regular "$installed_unit"
  run_service config validate
  active=false
  if service is-active --quiet "$unit"; then active=true; service stop "$unit"; fi
  replace_file "$source_dir/bin/$app" "$binary" 0755
  replace_file "$source_dir/systemd/$unit" "$installed_unit" 0644
  write_state_access
  service daemon-reload
  run_service check || fail 'new binary rejected current state; service remains stopped'
  if [[ "$active" == true ]]; then service start "$unit"; fi
  printf 'Upgraded %s; existing configuration and state were preserved.\n' "$app"
fi
