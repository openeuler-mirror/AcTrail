#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-release.sh [DESTDIR]

Install AcTrail release binaries into DESTDIR.
DESTDIR defaults to /usr/local/bin.

The script installs/checks build dependencies, runs cargo build --release when
target/release is incomplete, builds TLS sync preload runtimes, then copies the
release binaries and runtimes.

Environment:
  ACTRAIL_SUDO  Privilege command for installing into system directories.
                Defaults to sudo.
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

install_needs_privilege() {
  [[ "${EUID:-$(id -u)}" -ne 0 ]] || return 1
  if [[ -d "$dest_dir" ]]; then
    [[ ! -w "$dest_dir" ]]
    return
  fi
  [[ ! -w "$(dirname -- "$dest_dir")" ]]
}

install_prefix() {
  install_needs_privilege || return 0
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

mapfile -t privileged_install < <(install_prefix)

run "${privileged_install[@]}" install -d "$dest_dir"

for binary in "${binaries[@]}"; do
  source_path="$release_dir/$binary"
  if [[ ! -x "$source_path" ]]; then
    echo "missing executable $source_path; cargo build --release did not produce it" >&2
    exit 1
  fi
  run "${privileged_install[@]}" install -m 0755 "$source_path" "$dest_dir/$binary"
done

for runtime in "${runtimes[@]}"; do
  source_path="$release_dir/$runtime"
  if [[ ! -f "$source_path" ]]; then
    echo "missing runtime $source_path; scripts/build-tls-sync-runtimes.sh did not produce it" >&2
    exit 1
  fi
  run "${privileged_install[@]}" install -m 0755 "$source_path" "$dest_dir/$runtime"
done

printf 'installed AcTrail binaries to %s\n' "$dest_dir"
