#!/usr/bin/env bash
# Run virtual-container V2 cases with explicitly supplied configuration.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
RUNNER="$ROOT_DIR/tests/e2e/regression/test_all.py"
requested_selection=0

for argument in "$@"; do
  case "$argument" in
    --case|--case=*|--list)
      requested_selection=1
      ;;
  esac
done

selection=()
if [[ "$requested_selection" == "0" ]]; then
  selection=(
    --case virtual_container
    --case virtual_container_xiaoo_concurrency
  )
fi

runner_arguments=("$@")
if [[ "${#selection[@]}" -gt 0 ]]; then
  runner_arguments=("${selection[@]}" "${runner_arguments[@]}")
fi

resolve_user_home() {
  local user_name="$1"
  getent passwd "$user_name" 2>/dev/null | awk -F: 'NR == 1 { print $6 }'
}

if [[ "$(id -u)" != "0" ]]; then
  invoking_home="$HOME"
  invoking_path="$invoking_home/.local/bin:$invoking_home/.cargo/bin:$PATH"
  exec sudo -E env \
    "CARGO_HOME=${CARGO_HOME:-$invoking_home/.cargo}" \
    "RUSTUP_HOME=${RUSTUP_HOME:-$invoking_home/.rustup}" \
    "PATH=$invoking_path" \
    python3 "$RUNNER" "${runner_arguments[@]}"
fi

if [[ -n "${SUDO_USER:-}" && "$SUDO_USER" != "root" ]]; then
  invoking_home="$(resolve_user_home "$SUDO_USER")"
  if [[ -n "$invoking_home" ]]; then
    export CARGO_HOME="${CARGO_HOME:-$invoking_home/.cargo}"
    export RUSTUP_HOME="${RUSTUP_HOME:-$invoking_home/.rustup}"
    export PATH="$invoking_home/.local/bin:$invoking_home/.cargo/bin:$PATH"
  fi
fi

exec python3 "$RUNNER" "${runner_arguments[@]}"
