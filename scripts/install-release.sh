#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/install-release.sh [DESTDIR]

Install AcTrail release binaries from target/release into DESTDIR.
DESTDIR defaults to /usr/local/bin.

Build first with:
  cargo build --release
EOF
}

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
  actrailviewer
  actrailweb
)

if [[ ! -d "$release_dir" ]]; then
  echo "missing $release_dir; run cargo build --release first" >&2
  exit 1
fi

install -d "$dest_dir"

for binary in "${binaries[@]}"; do
  source_path="$release_dir/$binary"
  if [[ ! -x "$source_path" ]]; then
    echo "missing executable $source_path; run cargo build --release first" >&2
    exit 1
  fi
  install -m 0755 "$source_path" "$dest_dir/$binary"
done

printf 'installed AcTrail binaries to %s\n' "$dest_dir"
