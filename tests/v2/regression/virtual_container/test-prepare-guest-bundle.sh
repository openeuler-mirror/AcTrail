#!/usr/bin/env bash
# Regression checks for deterministic, target-compatible guest bundle assembly.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
PREPARE="$ROOT_DIR/tests/v2/regression/virtual_container/prepare-guest-bundle.sh"
TARGET_DIR="${ACTRAIL_BIN_DIR:-${CARGO_TARGET_DIR:-$ROOT_DIR/target}/release}"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-guest-bundle-test.XXXXXX")"
BUNDLE_DIR="$WORK_DIR/bundle"
chmod 1777 "$WORK_DIR"
PARENT_MODE="$(stat -c %a "$WORK_DIR")"

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

for artifact in \
  "$TARGET_DIR/actraild" \
  "$TARGET_DIR/actrailctl" \
  "$TARGET_DIR/actrailviewer" \
  "$TARGET_DIR/libactrail_tls_payload_probe_sync.so"; do
  [[ -f "$artifact" ]] \
    || fail "release artifact missing: $artifact; run cargo build --release first"
done

install -d "$BUNDLE_DIR"
printf 'stale\n' >"$BUNDLE_DIR/stale-from-previous-build"

PATH=/usr/bin:/bin \
BUNDLE_DIR="$BUNDLE_DIR" \
ACTRAIL_BUILD=0 \
COPY_OPENSSL=0 \
EBPF_TRANSPORT=perf-buffer \
  "$PREPARE" >/dev/null

[[ "$(stat -c %a "$WORK_DIR")" == "$PARENT_MODE" ]] \
  || fail "bundle preparation changed the existing parent directory mode"
[[ "$(stat -c %a "$BUNDLE_DIR")" == 755 ]] \
  || fail "published bundle root is not traversable by the workload user"
[[ ! -e "$BUNDLE_DIR/stale-from-previous-build" ]] \
  || fail "stale bundle file survived rebuild"
for library in libcrypto.so.3 libelf.so.1 libssl.so.3 libz.so.1 libzstd.so.1; do
  [[ -f "$BUNDLE_DIR/lib/$library" ]] || fail "dependency closure missing $library"
done
[[ ! -e "$BUNDLE_DIR/libelf.so.1" ]] || fail "runtime library leaked into bundle root"
[[ -f "$BUNDLE_DIR/BUNDLE-INFO" ]] || fail "BUNDLE-INFO missing"
grep -Eq '^program_interpreter=/' "$BUNDLE_DIR/BUNDLE-INFO" \
  || fail "BUNDLE-INFO has no absolute program_interpreter"
[[ -f "$BUNDLE_DIR/MANIFEST.sha256" ]] || fail "MANIFEST.sha256 missing"
for plugin_file in \
  otel-http.plugin.toml \
  otel-http.config.toml \
  otel-http.config.v1.schema.json; do
  [[ -f "$BUNDLE_DIR/plugins/otel-http/$plugin_file" ]] \
    || fail "OTLP/HTTP plugin asset missing: $plugin_file"
done
(cd "$BUNDLE_DIR" && sha256sum -c MANIFEST.sha256 >/dev/null)

actual_library_path="$(
  ACTRAIL_BUNDLE="$BUNDLE_DIR" \
    sh -c '. "$1"; printf "%s|%s\n" "$LD_LIBRARY_PATH" "$TLS_PAYLOAD_SYNC_LIBRARY"' sh \
    "$BUNDLE_DIR/tests/guest/common.sh"
)"
[[ "$actual_library_path" == "$BUNDLE_DIR/lib|$BUNDLE_DIR/libactrail_tls_payload_probe_sync.so" ]] \
  || fail "guest runtime library environment is incorrect: $actual_library_path"

ln -s "$BUNDLE_DIR" "$WORK_DIR/bundle-link"
set +e
symlink_output="$(
  BUNDLE_DIR="$WORK_DIR/bundle-link" \
  ACTRAIL_BUILD=0 \
  COPY_OPENSSL=0 \
    "$PREPARE" 2>&1
)"
symlink_rc=$?
set -e
[[ "$symlink_rc" -ne 0 ]] || fail "symbolic-link BUNDLE_DIR was accepted"
grep -q 'BUNDLE_DIR must not be a symbolic link' <<<"$symlink_output" \
  || fail "symbolic-link rejection did not explain the unsafe path"

manifest_before="$(sha256sum "$BUNDLE_DIR/MANIFEST.sha256")"
set +e
glibc_output="$(
  BUNDLE_DIR="$BUNDLE_DIR" \
  ACTRAIL_BUILD=0 \
  COPY_OPENSSL=0 \
  BUNDLE_TARGET_GLIBC=1.0 \
    "$PREPARE" 2>&1
)"
glibc_rc=$?
set -e
[[ "$glibc_rc" -ne 0 ]] || fail "incompatible GLIBC target was accepted"
grep -q 'requires GLIBC_.*target limit is GLIBC_1.0' <<<"$glibc_output" \
  || fail "GLIBC rejection did not explain the incompatibility"
[[ "$manifest_before" == "$(sha256sum "$BUNDLE_DIR/MANIFEST.sha256")" ]] \
  || fail "failed rebuild replaced the previous valid bundle"

FAKE_SYSROOT="$WORK_DIR/fake-sysroot"
install -d "$FAKE_SYSROOT/usr/lib"
ln -s /usr/lib "$FAKE_SYSROOT/lib"
set +e
sysroot_output="$(
  BUNDLE_DIR="$WORK_DIR/sysroot-escape-bundle" \
  BUNDLE_SYSROOT="$FAKE_SYSROOT" \
  BUNDLE_TARGET_GLIBC=99.0 \
  ACTRAIL_BUILD=0 \
  COPY_OPENSSL=0 \
    "$PREPARE" 2>&1
)"
sysroot_rc=$?
set -e
[[ "$sysroot_rc" -ne 0 ]] || fail "absolute sysroot library link escaped to host libraries"
grep -Eq \
  'actrailctl ELF interpreter is unavailable|cannot resolve dependency' \
  <<<"$sysroot_output" \
  || fail "sysroot escape rejection did not report a missing target runtime"

VALID_SYSROOT="$WORK_DIR/valid-sysroot"
install -d "$VALID_SYSROOT/lib" "$VALID_SYSROOT/usr/lib"
program_interpreter="$(
  sed -n 's/^program_interpreter=//p' "$BUNDLE_DIR/BUNDLE-INFO" | sed -n '1p'
)"
install -d "$VALID_SYSROOT$(dirname "$program_interpreter")"
install -m 0755 "$program_interpreter" "$VALID_SYSROOT$program_interpreter"
for library_path in "$BUNDLE_DIR"/lib/*; do
  library="$(basename "$library_path")"
  install -m 0644 "$library_path" "$VALID_SYSROOT/usr/lib/$library"
done
BUNDLE_DIR="$WORK_DIR/valid-sysroot-bundle" \
BUNDLE_SYSROOT="$VALID_SYSROOT" \
BUNDLE_TARGET_GLIBC=99.0 \
ACTRAIL_BUILD=0 \
COPY_OPENSSL=0 \
  "$PREPARE" >/dev/null
[[ -f "$WORK_DIR/valid-sysroot-bundle/lib/libelf.so.1" ]] \
  || fail "absolute sysroot library link did not resolve inside the target root"

echo "PREPARE_GUEST_BUNDLE_TEST_OK"
