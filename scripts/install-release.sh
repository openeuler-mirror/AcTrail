#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-release.sh [DESTDIR]

Install AcTrail release binaries into DESTDIR and the official plugin packages.
DESTDIR defaults to /usr/local/bin.

The script installs/checks build dependencies, runs cargo build --release when
target/release is incomplete, builds TLS sync preload runtimes, then copies the
release binaries, runtimes, and the installed-but-disabled file-leakage and
dynamic file-policy plugins.

Environment:
  ACTRAIL_SUDO  Privilege command for installing into system directories.
                Defaults to sudo.
  ACTRAIL_PLUGIN_DIR
                Plugin installation root. Defaults to ~/.actrail/plugins for
                the user running this script. The same absolute directory must
                be configured as plugins.discovery.directory.
EOF
}

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

if [[ $# -gt 1 ]]; then
  usage >&2
  exit 2
fi

dest_dir="${1:-/usr/local/bin}"
plugin_home="${HOME:?HOME is required to resolve the default plugin directory}"
plugin_root="${ACTRAIL_PLUGIN_DIR:-$plugin_home/.actrail/plugins}"
[[ "$plugin_root" = /* ]] || {
  echo "ACTRAIL_PLUGIN_DIR must be an absolute path: $plugin_root" >&2
  exit 2
}
file_leakage_install_dir="$plugin_root/file-leakage"
file_leakage_source_dir="$script_dir/../examples/plugins/wit-component/file-leakage"
file_leakage_artifact="$file_leakage_source_dir/target/wasm32-wasip2/release/actrail_file_leakage_plugin.wasm"
file_policy_install_dir="$plugin_root/file-policy-dynamic"
file_policy_source_dir="$script_dir/../examples/plugins/wit-component/file-policy-dynamic"
file_policy_fixture_dir="$file_policy_source_dir/fixture-src"
file_policy_artifact="$file_policy_fixture_dir/target/wasm32-wasip2/release/actrail_component_file_policy_dynamic.wasm"
release_dir="target/release"
binaries=(
  actraild
  actrailctl
  actrailcluster
  actrailviewer
  actrailweb
)
runtimes=(
  libactrail_tls_payload_probe_sync.so
  libactrail_tls_payload_probe_sync-musl.so
)

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

path_needs_privilege() {
  local path="$1"
  [[ "${EUID:-$(id -u)}" -ne 0 ]] || return 1
  if [[ -d "$path" ]]; then
    [[ ! -w "$path" ]]
    return
  fi
  [[ ! -w "$(dirname -- "$path")" ]]
}

install_prefix() {
  local target="$1"
  path_needs_privilege "$target" || return 0
  local configured_sudo="${ACTRAIL_SUDO:-sudo}"
  [[ -n "$configured_sudo" ]] || {
    echo "not root and ACTRAIL_SUDO is empty" >&2
    exit 1
  }

  local -a prefix
  read -r -a prefix <<<"$configured_sudo"
  command -v "${prefix[0]}" >/dev/null 2>&1 || {
    echo "privilege command '${prefix[0]}' is not on PATH" >&2
    exit 1
  }
  printf '%s\n' "${prefix[@]}"
}

"$script_dir/install-build-deps.sh" --install

missing_release_binary=0
for binary in "${binaries[@]}"; do
  if [[ ! -x "$release_dir/$binary" ]]; then
    missing_release_binary=1
  fi
done

if [[ "$missing_release_binary" -eq 1 ]]; then
  run cargo build --release \
    --bin actraild \
    --bin actrailctl \
    --bin actrailcluster \
    --bin actrailviewer \
    --bin actrailweb
fi

missing_runtime=0
for runtime in "${runtimes[@]}"; do
  if [[ ! -f "$release_dir/$runtime" ]]; then
    missing_runtime=1
  fi
done

if [[ "$missing_runtime" -eq 1 ]]; then
  run "$script_dir/build-tls-sync-runtimes.sh"
fi

command -v rustup >/dev/null 2>&1 || {
  echo "rustup is required to build the official wasm32-wasip2 plugin" >&2
  exit 1
}
if ! rustup target list --installed | grep -qx wasm32-wasip2; then
  run rustup target add wasm32-wasip2
fi
run cargo build --release --target wasm32-wasip2 \
  --manifest-path "$file_leakage_source_dir/Cargo.toml"
[[ -f "$file_leakage_artifact" ]] || {
  echo "missing plugin artifact $file_leakage_artifact" >&2
  exit 1
}
run cargo build --release --target wasm32-wasip2 \
  --manifest-path "$file_policy_fixture_dir/Cargo.toml"
[[ -f "$file_policy_artifact" ]] || {
  echo "missing plugin artifact $file_policy_artifact" >&2
  exit 1
}

mapfile -t binary_install < <(install_prefix "$dest_dir")
mapfile -t plugin_install < <(install_prefix "$plugin_root")

run "${binary_install[@]}" install -d "$dest_dir"
run "${plugin_install[@]}" install -d "$file_leakage_install_dir"
run "${plugin_install[@]}" install -d "$file_policy_install_dir"

for binary in "${binaries[@]}"; do
  source_path="$release_dir/$binary"
  if [[ ! -x "$source_path" ]]; then
    echo "missing executable $source_path; cargo build --release did not produce it" >&2
    exit 1
  fi
  run "${binary_install[@]}" install -m 0755 "$source_path" "$dest_dir/$binary"
done

for runtime in "${runtimes[@]}"; do
  source_path="$release_dir/$runtime"
  if [[ ! -f "$source_path" ]]; then
    echo "missing runtime $source_path; scripts/build-tls-sync-runtimes.sh did not produce it" >&2
    exit 1
  fi
  run "${binary_install[@]}" install -m 0755 "$source_path" "$dest_dir/$runtime"
done

for asset in \
  file-leakage.plugin.toml \
  file-leakage.config.json \
  file-leakage.config.v1.schema.json \
  file-leakage.payload.v1.schema.json; do
  run "${plugin_install[@]}" install -m 0644 \
    "$file_leakage_source_dir/$asset" "$file_leakage_install_dir/$asset"
done
run "${plugin_install[@]}" install -m 0644 \
  "$file_leakage_artifact" \
  "$file_leakage_install_dir/actrail_file_leakage_plugin.wasm"
run "${plugin_install[@]}" install -m 0644 \
  "$file_policy_source_dir/plugin.toml" \
  "$file_policy_install_dir/file-policy-dynamic.plugin.toml"
for asset in \
  file-policy-dynamic.config.json \
  config.schema.json; do
  run "${plugin_install[@]}" install -m 0644 \
    "$file_policy_source_dir/$asset" "$file_policy_install_dir/$asset"
done
run "${plugin_install[@]}" install -m 0644 \
  "$file_policy_artifact" \
  "$file_policy_install_dir/component-file-policy-dynamic.wasm"

printf 'installed AcTrail binaries to %s and plugins to %s\n' "$dest_dir" "$plugin_root"
