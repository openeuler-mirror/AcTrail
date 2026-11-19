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
source "$repo_root/scripts/ci/lib/common.sh"
cd "$repo_root"
target_dir="${CARGO_TARGET_DIR:-target}"
mkdir -p "$target_dir"
build_log_dir="$(mktemp -d "$target_dir/build-logs.XXXXXX")"
trap 'rm -r -- "$build_log_dir"' EXIT
trap 'exit 130' INT
trap 'exit 143' TERM
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

# Use installed target libraries directly; use rustup only for missing targets.
if [[ -z "$prebuilt_musl_runtime" ]]; then
  target_libdir="$(rustc --print target-libdir --target "$musl_target")"
  if ! compgen -G "$target_libdir/libstd-*.rlib" >/dev/null; then
    command -v rustup >/dev/null 2>&1 || {
      ci_log SKIP "$musl_target standard library missing in $target_libdir; rustup not found on PATH"
      exit 1
    }
    ci_log INFO "Installing Rust target: $musl_target"
    if run rustup target add "$musl_target"; then
      ci_log PASS "rustup target add $musl_target completed"
    else
      code=$?
      ci_log SKIP "rustup target add $musl_target failed (exit $code); see command output above"
      exit "$code"
    fi
  fi
fi

if [[ "$musl_only" -eq 0 ]]; then
  # Host runtime comes from the plain full-workspace build so fingerprints
  # stay identical to a normal `cargo build --release` (no -p flip-flop).
  ci_log INFO 'Starting release build'
  if cargo build --release >"$build_log_dir/workspace.log" 2>&1; then
    rm -- "$build_log_dir/workspace.log"
    ci_log PASS 'Release build completed'
  else
    code=$?
    ci_log FAIL "Release build failed (exit $code)"
    cat "$build_log_dir/workspace.log" >&2
    rm -- "$build_log_dir/workspace.log"
    exit "$code"
  fi
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
  ci_log INFO 'Starting musl TLS build'
  if cargo build --release -p tls_payload_probe_sync --target "$musl_target" --lib \
      >"$build_log_dir/tls-$musl_target.log" 2>&1; then
    rm -- "$build_log_dir/tls-$musl_target.log"
    ci_log PASS 'Musl TLS build completed'
  else
    code=$?
    ci_log FAIL "Musl TLS build failed (exit $code)"
    cat "$build_log_dir/tls-$musl_target.log" >&2
    rm -- "$build_log_dir/tls-$musl_target.log"
    exit "$code"
  fi

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
