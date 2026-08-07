#!/usr/bin/env bash
# Build the daemon-free bundle mounted read-only into a Kata workload.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
ROOT_DIR="$(cd -- "$SCRIPT_DIR/../../.." && pwd)"
GUEST_BUNDLE="${BUNDLE_DIR:-$ROOT_DIR/.actrail-guest-bundle}"
OUTPUT="${WORKLOAD_BUNDLE_DIR:-$ROOT_DIR/.actrail-workload-bundle}"
GUEST_CONFIG="$ROOT_DIR/deploy/virtual-container/guest/operator.conf"
SOCKET_GID=39000

usage() {
  cat <<'EOF'
Usage: prepare-bundle.sh [options]

Options:
  --guest-bundle DIR  Verified guest bundle containing actrailctl/probe/libs
  --output DIR        Output directory (default: .actrail-workload-bundle)
  --socket-gid GID    Supplemental GID used by the guest sockets (default: 39000)
  -h, --help          Show this help

The output intentionally excludes actraild, storage, logs and viewer binaries.
Mount it read-only at /opt/actrail in the workload.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --guest-bundle)
      [[ "$#" -ge 2 ]] || fail "--guest-bundle requires a value"
      GUEST_BUNDLE="$2"
      shift 2
      ;;
    --output)
      [[ "$#" -ge 2 ]] || fail "--output requires a value"
      OUTPUT="$2"
      shift 2
      ;;
    --socket-gid)
      [[ "$#" -ge 2 ]] || fail "--socket-gid requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--socket-gid must be an integer"
      SOCKET_GID="$((10#$2))"
      shift 2
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      fail "unknown argument: $1"
      ;;
  esac
done

for command_name in \
  awk basename chmod dirname find grep install mkdir mktemp mv realpath rm \
  readelf sed sha256sum sort; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing command: $command_name"
done

(( SOCKET_GID > 0 && SOCKET_GID <= 2147483647 )) \
  || fail "--socket-gid must be between 1 and 2147483647"
[[ -d "$GUEST_BUNDLE" ]] || fail "guest bundle is not a directory: $GUEST_BUNDLE"
[[ -f "$GUEST_CONFIG" ]] || fail "guest operator config not found: $GUEST_CONFIG"

GUEST_BUNDLE="$(realpath "$GUEST_BUNDLE")"
FINAL_OUTPUT="$(realpath -ms "$OUTPUT")"
case "$FINAL_OUTPUT" in
  /|'') fail "unsafe output: $FINAL_OUTPUT" ;;
esac
[[ ! -L "$FINAL_OUTPUT" ]] || fail "output must not be a symbolic link: $FINAL_OUTPUT"

manifest="$GUEST_BUNDLE/MANIFEST.sha256"
[[ -f "$manifest" ]] || fail "guest bundle manifest not found: $manifest"
awk '
  NF < 2 || $2 !~ /^\.\// || $2 ~ /(^|\/)\.\.(\/|$)/ { exit 1 }
' "$manifest" || fail "guest bundle manifest contains an unsafe path"
(
  cd "$GUEST_BUNDLE"
  sha256sum --strict --check MANIFEST.sha256 >/dev/null
) || fail "guest bundle checksum verification failed"

for relative in actrailctl libactrail_tls_payload_probe_sync.so BUNDLE-INFO; do
  [[ -f "$GUEST_BUNDLE/$relative" ]] || fail "guest bundle file missing: $relative"
done
[[ -d "$GUEST_BUNDLE/lib" ]] || fail "guest bundle library directory is missing"

PROGRAM_INTERPRETER="$(
  readelf -l "$GUEST_BUNDLE/actrailctl" 2>/dev/null \
    | sed -n 's/.*Requesting program interpreter: \([^]]*\).*/\1/p'
)"
case "$PROGRAM_INTERPRETER" in
  /*) ;;
  *) fail "guest actrailctl has no absolute ELF interpreter" ;;
esac
recorded_interpreter="$(
  awk -F= '$1 == "program_interpreter" { print substr($0, index($0, "=") + 1); exit }' \
    "$GUEST_BUNDLE/BUNDLE-INFO"
)"
if [[ -n "$recorded_interpreter" && "$recorded_interpreter" != "$PROGRAM_INTERPRETER" ]]; then
  fail "guest bundle interpreter metadata does not match actrailctl"
fi

output_parent="$(dirname "$FINAL_OUTPUT")"
mkdir -p "$output_parent"
STAGING_DIR="$(mktemp -d "$output_parent/.actrail-workload-bundle.XXXXXX")"
chmod 0755 "$STAGING_DIR"
cleanup_staging() {
  [[ -n "${STAGING_DIR:-}" && -d "$STAGING_DIR" ]] && rm -rf "$STAGING_DIR"
}
trap cleanup_staging EXIT

install -d -m 0755 \
  "$STAGING_DIR/bin" \
  "$STAGING_DIR/etc/actrail" \
  "$STAGING_DIR/lib"
install -m 0755 "$GUEST_BUNDLE/actrailctl" "$STAGING_DIR/bin/actrailctl"
install -m 0755 \
  "$GUEST_BUNDLE/libactrail_tls_payload_probe_sync.so" \
  "$STAGING_DIR/lib/libactrail_tls_payload_probe_sync.so"
while IFS= read -r library; do
  install -m 0644 "$library" "$STAGING_DIR/lib/$(basename "$library")"
done < <(find "$GUEST_BUNDLE/lib" -maxdepth 1 -type f -print | LC_ALL=C sort)
install -m 0755 "$SCRIPT_DIR/actrail-launch" "$STAGING_DIR/bin/actrail-launch"
install -m 0755 "$SCRIPT_DIR/actrail-init" "$STAGING_DIR/bin/actrail-init"
install -m 0755 "$SCRIPT_DIR/actrailctl-private" "$STAGING_DIR/bin/actrailctl-private"
install -m 0755 "$SCRIPT_DIR/verify-interface.sh" "$STAGING_DIR/bin/verify-interface"

sed \
  -e 's|socket_path = "/dev/actrail/control.sock"|socket_path = "/run/actrail/control.sock"|' \
  -e 's|sync_event_socket_path = "/dev/actrail/tls-sync.sock"|sync_event_socket_path = "/run/actrail/tls-sync.sock"|' \
  -e 's|sync_runtime_library_path = "/usr/local/lib/actrail/libactrail_tls_payload_probe_sync.so"|sync_runtime_library_path = "/opt/actrail/lib/libactrail_tls_payload_probe_sync.so"|' \
  "$GUEST_CONFIG" >"$STAGING_DIR/etc/actrail/operator.conf"
chmod 0644 "$STAGING_DIR/etc/actrail/operator.conf"

grep -Fqx 'socket_path = "/run/actrail/control.sock"' \
  "$STAGING_DIR/etc/actrail/operator.conf" \
  || fail "failed to rewrite the workload control socket"
grep -Fqx 'sync_event_socket_path = "/run/actrail/tls-sync.sock"' \
  "$STAGING_DIR/etc/actrail/operator.conf" \
  || fail "failed to rewrite the workload TLS socket"
grep -Fqx \
  'sync_runtime_library_path = "/opt/actrail/lib/libactrail_tls_payload_probe_sync.so"' \
  "$STAGING_DIR/etc/actrail/operator.conf" \
  || fail "failed to rewrite the workload TLS probe path"

cat >"$STAGING_DIR/WORKLOAD-INTERFACE" <<EOF
format=1
guest_socket_source=/dev/actrail
workload_socket_target=/run/actrail
socket_group=actrail
socket_gid=$SOCKET_GID
socket_mode=0660
socket_directory_mode=0750
tool_target=/opt/actrail
program_interpreter=$PROGRAM_INTERPRETER
control_socket=/run/actrail/control.sock
tls_socket=/run/actrail/tls-sync.sock
EOF
chmod 0644 "$STAGING_DIR/WORKLOAD-INTERFACE"

{
  printf 'format=1\n'
  sed -n \
    -e 's/^target_machine=/target_machine=/p' \
    -e 's/^target_glibc_max=/target_glibc_max=/p' \
    -e 's/^build_profile=/build_profile=/p' \
    "$GUEST_BUNDLE/BUNDLE-INFO"
  printf 'program_interpreter=%s\n' "$PROGRAM_INTERPRETER"
  printf 'source_guest_manifest_sha256=%s\n' \
    "$(sha256sum "$GUEST_BUNDLE/MANIFEST.sha256" | awk '{ print $1 }')"
} >"$STAGING_DIR/BUNDLE-INFO"
chmod 0644 "$STAGING_DIR/BUNDLE-INFO"

(
  cd "$STAGING_DIR"
  while IFS= read -r path; do
    sha256sum "$path"
  done < <(find . -type f ! -name MANIFEST.sha256 -print | LC_ALL=C sort)
) >"$STAGING_DIR/MANIFEST.sha256"
chmod 0644 "$STAGING_DIR/MANIFEST.sha256"

# Parse the generated config with the real client before publishing it.
LD_LIBRARY_PATH="$STAGING_DIR/lib" \
  "$STAGING_DIR/bin/actrailctl" \
  --config "$STAGING_DIR/etc/actrail/operator.conf" \
  probe --skip-daemon --json >/dev/null

backup_dir="${FINAL_OUTPUT}.previous.$$"
[[ ! -e "$backup_dir" ]] || fail "temporary backup path already exists: $backup_dir"
if [[ -e "$FINAL_OUTPUT" ]]; then
  mv "$FINAL_OUTPUT" "$backup_dir"
fi
if mv "$STAGING_DIR" "$FINAL_OUTPUT"; then
  STAGING_DIR=""
  rm -rf "$backup_dir"
else
  [[ ! -e "$FINAL_OUTPUT" && -e "$backup_dir" ]] \
    && mv "$backup_dir" "$FINAL_OUTPUT"
  fail "failed to publish workload bundle at $FINAL_OUTPUT"
fi
trap - EXIT

echo "ACTRAIL_WORKLOAD_BUNDLE_READY"
echo "output=$FINAL_OUTPUT"
echo "socket_gid=$SOCKET_GID"
