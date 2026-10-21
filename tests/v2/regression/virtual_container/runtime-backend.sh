#!/usr/bin/env bash
# Shared Kata runtime configuration resolution for the V2 virtual-container case.

command -v python3 >/dev/null 2>&1 || {
  echo "FAIL: missing command: python3" >&2
  return 1
}
HOST_DEFAULTS="$(python3 "$(dirname -- "${BASH_SOURCE[0]}")/constants.py")" || return 1
eval "$HOST_DEFAULTS"
unset HOST_DEFAULTS

kata_shim_binary_for_runtime() {
  local runtime="$1"
  local runtime_name=""

  case "$runtime" in
    io.containerd.*.v2)
      runtime_name="${runtime#io.containerd.}"
      runtime_name="${runtime_name%.v2}"
      ;;
    *)
      echo "unsupported containerd runtime name: $runtime" >&2
      return 1
      ;;
  esac
  [[ "$runtime_name" =~ ^[[:alnum:]_-]+$ ]] || {
    echo "invalid containerd runtime component: $runtime_name" >&2
    return 1
  }
  printf 'containerd-shim-%s-v2\n' "$runtime_name"
}

default_kata_ctr_runtime() {
  if command -v "$KATA_PREFERRED_SHIM_BINARY" >/dev/null 2>&1; then
    printf '%s\n' "$KATA_PREFERRED_CTR_RUNTIME"
  else
    printf '%s\n' "$KATA_DEFAULT_CTR_RUNTIME"
  fi
}

kata_runtime_config_name() {
  case "$1" in
    stratovirt) printf '%s\n' "$KATA_STRATOVIRT_CONFIG_NAME" ;;
    cloud-hypervisor) printf '%s\n' "$KATA_CLOUD_HYPERVISOR_CONFIG_NAME" ;;
    default) return 0 ;;
    *)
      echo "unsupported BACKEND=$1 (expected stratovirt, cloud-hypervisor or default)" >&2
      return 1
      ;;
  esac
}

find_kata_runtime_config() {
  local backend="$1"
  local name=""
  local config_dirs=""
  local config_dir=""
  local -a dirs=()

  name="$(kata_runtime_config_name "$backend")" || return 1
  [[ -n "$name" ]] || return 0

  config_dirs="$KATA_CONFIG_DIRS"
  IFS=: read -r -a dirs <<<"$config_dirs"
  for config_dir in "${dirs[@]}"; do
    [[ -n "$config_dir" ]] || continue
    if [[ -f "$config_dir/$name" ]]; then
      printf '%s\n' "$config_dir/$name"
      return 0
    fi
  done

  echo "Kata runtime config $name was not found in KATA_CONFIG_DIRS=$config_dirs" >&2
  return 1
}

resolve_kata_runtime_config() {
  local backend="$1"
  local explicit_path="$2"

  if [[ -n "$explicit_path" ]]; then
    if [[ ! -f "$explicit_path" ]]; then
      echo "runtime config not found: $explicit_path" >&2
      return 1
    fi
    printf '%s\n' "$explicit_path"
    return 0
  fi

  find_kata_runtime_config "$backend"
}
