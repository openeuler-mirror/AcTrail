#!/usr/bin/env bash
# Run both virtual-container V2 cases with the machine-local profile.
set -euo pipefail

ROOT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../.." && pwd)"
RUNNER="$ROOT_DIR/tests/v2/regression/test_all.py"
requested_selection=0
profile_selection=default
list_only=0

for argument in "$@"; do
  case "$argument" in
    --case|--case=*|--list)
      requested_selection=1
      ;;
  esac
  case "$argument" in
    --profile|--profile=*|--no-profile)
      profile_selection=explicit
      ;;
    --list)
      list_only=1
      ;;
  esac
done

default_profile="$ROOT_DIR/local/kata/v2-test-profile.json"
profile_arguments=()
if [[ "$list_only" == "0" \
  && "$profile_selection" == "default" \
  && ! -f "$default_profile" ]]; then
  requested_scope="${VIRTUAL_CONTAINER_E2E_SCOPE:-auto}"
  if [[ "$requested_scope" == "contracts" \
    || ( "$requested_scope" == "auto" && ! -c /dev/kvm ) ]]; then
    profile_arguments=(--no-profile)
  else
    cat >&2 <<EOF
error: missing machine-local V2 profile: $default_profile
Prepare the Kata artifacts from this same checkout before running V2 tests:
  python3 deploy/virtual-container/host/prepare-v2-test-artifacts.py --help
See deploy/virtual-container/README.md for the first-deployment command.
EOF
    exit 2
  fi
fi

selection=()
if [[ "$requested_selection" == "0" ]]; then
  selection=(
    --case virtual_container
    --case virtual_container_xiaoo_concurrency
  )
fi

runner_arguments=("$@")
if [[ "${#profile_arguments[@]}" -gt 0 ]]; then
  runner_arguments=("${profile_arguments[@]}" "${runner_arguments[@]}")
fi
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
