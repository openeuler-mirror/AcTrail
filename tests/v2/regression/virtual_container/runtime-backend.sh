#!/usr/bin/env bash
# Shared Kata runtime configuration resolution for the V2 virtual-container case.

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
  # A versioned shim name avoids an older distro-owned
  # /usr/bin/containerd-shim-kata-v2 winning containerd's PATH lookup. Keep
  # the standard name as a compatibility fallback for existing test hosts.
  if command -v containerd-shim-kata332-v2 >/dev/null 2>&1; then
    printf '%s\n' 'io.containerd.kata332.v2'
  else
    printf '%s\n' 'io.containerd.kata.v2'
  fi
}

kata_runtime_config_name() {
  case "$1" in
    stratovirt) printf '%s\n' "configuration-stratovirt.toml" ;;
    cloud-hypervisor) printf '%s\n' "configuration-clh.toml" ;;
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

  config_dirs="${KATA_CONFIG_DIRS:-/opt/kata/share/defaults/kata-containers:/usr/share/defaults/kata-containers:/etc/kata-containers}"
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
