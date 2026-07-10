#!/usr/bin/env bash
set -euo pipefail

usage() {
  cat <<'EOF'
Usage: scripts/build-tls-sync-runtimes.sh [--musl-only]

Build TLS sync preload runtimes for the host glibc ABI and the matching musl ABI.

Environment:
  ACTRAIL_MUSL_TARGET   Override musl Rust target triple.
  ACTRAIL_MUSL_LINKER   Override musl C linker.
  ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME
                         Install this prebuilt musl runtime instead of building it.
EOF
}

if [[ "${1:-}" == "-h" || "${1:-}" == "--help" ]]; then
  usage
  exit 0
fi

musl_only=0
if [[ "${1:-}" == "--musl-only" ]]; then
  musl_only=1
  shift
fi

if [[ $# -ne 0 ]]; then
  usage >&2
  exit 2
fi

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$repo_root"
target_dir="${CARGO_TARGET_DIR:-target}"
runtime_output_dir="${ACTRAIL_TLS_SYNC_RUNTIME_OUTPUT_DIR:-$target_dir/release}"
prebuilt_musl_runtime="${ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME:-}"

host_triple="$(rustc -vV | awk '/^host:/ { print $2 }')"
case "${ACTRAIL_MUSL_TARGET:-$host_triple}" in
  x86_64-unknown-linux-gnu | x86_64-unknown-linux-musl)
    musl_target="${ACTRAIL_MUSL_TARGET:-x86_64-unknown-linux-musl}"
    linker_candidates=(x86_64-linux-musl-gcc musl-gcc)
    linker_env=CARGO_TARGET_X86_64_UNKNOWN_LINUX_MUSL_LINKER
    ;;
  aarch64-unknown-linux-gnu | aarch64-unknown-linux-musl)
    musl_target="${ACTRAIL_MUSL_TARGET:-aarch64-unknown-linux-musl}"
    linker_candidates=(aarch64-linux-musl-gcc musl-gcc)
    linker_env=CARGO_TARGET_AARCH64_UNKNOWN_LINUX_MUSL_LINKER
    ;;
  *)
    echo "unsupported host for TLS sync musl runtime: $host_triple" >&2
    exit 1
    ;;
esac
printf 'tls_sync_host_triple=%s\n' "$host_triple"
printf 'tls_sync_musl_target=%s\n' "$musl_target"

musl_linker="${ACTRAIL_MUSL_LINKER:-}"
if [[ -n "$prebuilt_musl_runtime" ]]; then
  musl_linker=prebuilt
elif [[ -z "$musl_linker" ]]; then
  for candidate in "${linker_candidates[@]}"; do
    if command -v "$candidate" >/dev/null 2>&1; then
      musl_linker="$candidate"
      break
    fi
  done
fi
if [[ -z "$musl_linker" ]]; then
  echo "missing musl linker for $musl_target; searched: ${linker_candidates[*]}" >&2
  echo "install a musl cross compiler, set ACTRAIL_MUSL_LINKER, or set ACTRAIL_TLS_SYNC_PREBUILT_MUSL_RUNTIME" >&2
  exit 1
fi

run() {
  printf '+'
  printf ' %q' "$@"
  printf '\n'
  "$@"
}

if [[ -n "$prebuilt_musl_runtime" ]]; then
  :
elif ! command -v rustup >/dev/null 2>&1; then
  echo "missing rustup; cannot verify or install Rust musl target $musl_target" >&2
  exit 1
fi
if [[ -z "$prebuilt_musl_runtime" ]] && ! rustup target list --installed | grep -qx "$musl_target"; then
  run rustup target add "$musl_target"
fi

if [[ "$musl_only" -eq 0 ]]; then
  run cargo build --release -p tls_payload_probe_sync --lib
fi

musl_output="$runtime_output_dir/libactrail_tls_payload_probe_sync-musl.so"
if [[ -n "$prebuilt_musl_runtime" ]]; then
  if [[ ! -f "$prebuilt_musl_runtime" ]]; then
    echo "prebuilt musl runtime is not a file: $prebuilt_musl_runtime" >&2
    exit 1
  fi
  run mkdir -p "$(dirname -- "$musl_output")"
  run install -m 0755 "$prebuilt_musl_runtime" "$musl_output"
else
  shim_dir="$target_dir/$musl_target/tls-sync-linker-shim"
  run mkdir -p "$shim_dir"
  printf '/* Intentionally empty: satisfy rustc cdylib -lgcc_s without pulling libgcc unwind symbols. */\n' >"$shim_dir/libgcc_s.so"

  export "$linker_env=$musl_linker"
  unset CARGO_ENCODED_RUSTFLAGS
  export RUSTFLAGS="${RUSTFLAGS:-} -C target-feature=-crt-static -C panic=abort -L native=$shim_dir"
  run cargo build --release -p tls_payload_probe_sync --target "$musl_target" --lib

  musl_source="$target_dir/$musl_target/release/libactrail_tls_payload_probe_sync.so"
  if [[ ! -f "$musl_source" ]]; then
    echo "missing musl runtime output $musl_source" >&2
    exit 1
  fi
  run mkdir -p "$(dirname -- "$musl_output")"
  run install -m 0755 "$musl_source" "$musl_output"
fi

needed="$(readelf -d "$musl_output" | grep 'NEEDED' || true)"
if grep -Eq 'libc\.so\.6|ld-linux|libgcc_s' <<<"$needed"; then
  echo "musl TLS sync runtime unexpectedly depends on glibc/libgcc_s:" >&2
  printf '%s\n' "$needed" >&2
  exit 1
fi
undefined="$(readelf -Ws "$musl_output" | grep ' UND ' || true)"
if grep -Eq '_dl_find_object|_Unwind_' <<<"$undefined"; then
  echo "musl TLS sync runtime has unsupported unresolved unwind/glibc symbols:" >&2
  printf '%s\n' "$undefined" >&2
  exit 1
fi

printf 'built TLS sync runtimes:\n'
printf '  %s/libactrail_tls_payload_probe_sync.so\n' "$runtime_output_dir"
printf '  %s\n' "$musl_output"
