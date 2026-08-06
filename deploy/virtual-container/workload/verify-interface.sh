#!/bin/sh
# Verify the daemon-free workload artifacts and the mounted guest socket interface.
set -eu

ROOT="${ACTRAIL_WORKLOAD_ROOT:-/opt/actrail}"
SOCKET_DIR="${ACTRAIL_SOCKET_DIR:-}"
ARTIFACTS_ONLY=0

usage() {
  cat <<'EOF'
Usage: verify-interface [--root DIR] [--socket-dir DIR] [--artifacts-only]

Normal mode is intended to run inside a Kata workload. It requires:
  * the workload bundle mounted read-only at DIR;
  * guest socket source mounted read-only at SOCKET_DIR;
  * the configured supplemental socket GID on the workload process.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [ "$#" -gt 0 ]; do
  case "$1" in
    --root)
      [ "$#" -ge 2 ] || fail "--root requires a value"
      ROOT="$2"
      shift 2
      ;;
    --socket-dir)
      [ "$#" -ge 2 ] || fail "--socket-dir requires a value"
      SOCKET_DIR="$2"
      shift 2
      ;;
    --artifacts-only)
      ARTIFACTS_ONLY=1
      shift
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

[ "$ROOT" != "/" ] || fail "workload root must not be /"
[ -d "$ROOT" ] || fail "workload root is not a directory: $ROOT"
ROOT="${ROOT%/}"
for command_name in awk id sed sha256sum stat; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "missing command: $command_name"
done
for relative in \
  bin/actrailctl \
  bin/actrailctl-private \
  bin/actrail-init \
  bin/actrail-launch \
  etc/actrail/operator.conf \
  lib/libactrail_tls_payload_probe_sync.so \
  WORKLOAD-INTERFACE \
  MANIFEST.sha256; do
  [ -f "$ROOT/$relative" ] || fail "workload artifact missing: $relative"
done
[ -x "$ROOT/bin/actrailctl" ] || fail "actrailctl is not executable"
[ -x "$ROOT/bin/actrailctl-private" ] || fail "actrailctl-private is not executable"
[ -x "$ROOT/bin/actrail-init" ] || fail "actrail-init is not executable"
[ -x "$ROOT/bin/actrail-launch" ] || fail "actrail-launch is not executable"

(
  cd "$ROOT"
  sha256sum --strict --check MANIFEST.sha256 >/dev/null
) || fail "workload bundle checksum verification failed"
if awk '
  {
    path = $2
    sub(/^\*/, "", path)
    sub(/^\.\//, "", path)
    count = split(path, components, "/")
    basename = components[count]
    if (basename == "actraild" || basename == "actrailviewer") {
      found = 1
    }
  }
  END { exit !found }
' "$ROOT/MANIFEST.sha256"; then
  fail "workload bundle contains a system daemon or viewer"
fi

field() {
  key="$1"
  sed -n "s/^${key}=//p" "$ROOT/WORKLOAD-INTERFACE" | sed -n '1p'
}

[ "$(field format)" = "1" ] || fail "unsupported workload interface format"
EXPECTED_GID="$(field socket_gid)"
case "$EXPECTED_GID" in
  ''|*[!0-9]*) fail "invalid socket_gid in WORKLOAD-INTERFACE" ;;
esac
if [ -z "$SOCKET_DIR" ]; then
  SOCKET_DIR="$(field workload_socket_target)"
fi
case "$SOCKET_DIR" in
  /*) ;;
  *) fail "socket directory must be absolute" ;;
esac
[ "$SOCKET_DIR" != "/" ] || fail "socket directory must not be /"
SOCKET_DIR="${SOCKET_DIR%/}"

if [ "$ARTIFACTS_ONLY" = "1" ]; then
  echo "ACTRAIL_WORKLOAD_ARTIFACTS_OK"
  exit 0
fi

case " $(id -G) " in
  *" $EXPECTED_GID "*) ;;
  *) fail "workload process is missing supplemental GID $EXPECTED_GID" ;;
esac

[ -d "$SOCKET_DIR" ] || fail "socket directory is not mounted: $SOCKET_DIR"
[ -S "$SOCKET_DIR/control.sock" ] || fail "control socket is missing"
[ -S "$SOCKET_DIR/tls-sync.sock" ] || fail "TLS sync socket is missing"

[ "$(stat -c '%a' "$SOCKET_DIR")" = "750" ] \
  || fail "socket directory mode is not 0750"
for socket in "$SOCKET_DIR/control.sock" "$SOCKET_DIR/tls-sync.sock"; do
  [ "$(stat -c '%a' "$socket")" = "660" ] \
    || fail "socket mode is not 0660: $socket"
  [ "$(stat -c '%g' "$socket")" = "$EXPECTED_GID" ] \
    || fail "socket GID does not match $EXPECTED_GID: $socket"
done

require_readonly_mount() {
  target="$1"
  mount_options="$(
    awk -v target="$target" '$5 == target { print $6; exit }' /proc/self/mountinfo
  )"
  [ -n "$mount_options" ] || fail "$target is not a distinct mount"
  case ",$mount_options," in
    *,ro,*) ;;
    *) fail "$target must be mounted read-only" ;;
  esac
}

require_readonly_mount "$ROOT"
require_readonly_mount "$SOCKET_DIR"

"$ROOT/bin/actrailctl-private" \
  --config "$ROOT/etc/actrail/operator.conf" \
  doctor >/dev/null
"$ROOT/bin/actrailctl-private" \
  --config "$ROOT/etc/actrail/operator.conf" \
  probe --json >/dev/null

echo "ACTRAIL_WORKLOAD_INTERFACE_OK"
