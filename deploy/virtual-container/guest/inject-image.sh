#!/usr/bin/env bash
# Copy a partitioned Kata rootfs image and inject the AcTrail guest service.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=otel-endpoint.sh
source "$SCRIPT_DIR/otel-endpoint.sh"
SOURCE_IMAGE=""
OUTPUT_IMAGE=""
BUNDLE=""
OTEL_ENDPOINT=""
STARTUP_DEPENDENCY="optional"
WITH_VIEWER=0
SOCKET_GID=39000
GROW_MIB=0
LOOP_DEVICE=""
MOUNT_DIR=""

usage() {
  cat <<'EOF'
Usage: inject-image.sh --source-image FILE --output-image FILE --bundle DIR --otel-endpoint URL [options]

Options:
  --otel-endpoint URL          Guest-reachable OTLP/HTTP traces URL (required)
  --startup-dependency POLICY  optional or required (default: optional)
  --with-viewer                Also install actrailviewer into the guest image
  --socket-gid GID             Numeric GID shared with workloads (default: 39000)
  --grow-mib N                 Grow the copied image and ext4 rootfs by N MiB
  -h, --help                   Show this help

The source image is never mounted or modified. The output path must not exist.
The image may contain an ext4 filesystem directly or in partition 1.

The OTLP endpoint must use http:// or https:// and name the Collector address
reachable from inside the Guest. Guest 127.0.0.1 is the Guest itself, not the
host. Its path must end in /v1/traces; query strings, fragments and placeholder
endpoints are rejected before the output image is copied.

The startup dependency controls only kata-agent systemd ordering/readiness.
It does not select Agent observation failure behavior.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --source-image)
      [[ "$#" -ge 2 ]] || fail "--source-image requires a value"
      SOURCE_IMAGE="$2"
      shift 2
      ;;
    --output-image)
      [[ "$#" -ge 2 ]] || fail "--output-image requires a value"
      OUTPUT_IMAGE="$2"
      shift 2
      ;;
    --bundle)
      [[ "$#" -ge 2 ]] || fail "--bundle requires a value"
      BUNDLE="$2"
      shift 2
      ;;
    --otel-endpoint)
      [[ "$#" -ge 2 ]] || fail "--otel-endpoint requires a value"
      OTEL_ENDPOINT="$2"
      shift 2
      ;;
    --startup-dependency)
      [[ "$#" -ge 2 ]] || fail "--startup-dependency requires a value"
      STARTUP_DEPENDENCY="$2"
      shift 2
      ;;
    --with-viewer)
      WITH_VIEWER=1
      shift
      ;;
    --socket-gid)
      [[ "$#" -ge 2 ]] || fail "--socket-gid requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--socket-gid must be an integer"
      SOCKET_GID="$((10#$2))"
      shift 2
      ;;
    --grow-mib)
      [[ "$#" -ge 2 ]] || fail "--grow-mib requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--grow-mib must be an integer"
      GROW_MIB="$((10#$2))"
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

[[ -n "$SOURCE_IMAGE" ]] || fail "--source-image is required"
[[ -n "$OUTPUT_IMAGE" ]] || fail "--output-image is required"
[[ -n "$BUNDLE" ]] || fail "--bundle is required"
actrail_validate_guest_otel_endpoint "$OTEL_ENDPOINT" \
  || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
[[ -f "$SOURCE_IMAGE" ]] || fail "source image not found: $SOURCE_IMAGE"
[[ -d "$BUNDLE" ]] || fail "bundle not found: $BUNDLE"
case "$STARTUP_DEPENDENCY" in
  optional|required) ;;
  *) fail "--startup-dependency must be optional or required" ;;
esac
(( SOCKET_GID > 0 && SOCKET_GID <= 2147483647 )) \
  || fail "--socket-gid must be between 1 and 2147483647"
(( GROW_MIB >= 0 && GROW_MIB <= 4096 )) \
  || fail "--grow-mib must be between 0 and 4096"

for command_name in cp losetup mktemp mount mountpoint readlink rmdir seq sleep sync umount; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing command: $command_name"
done
if (( GROW_MIB > 0 )); then
  for command_name in e2fsck parted partprobe resize2fs truncate; do
    command -v "$command_name" >/dev/null 2>&1 \
      || fail "--grow-mib requires command: $command_name"
  done
fi

SOURCE_IMAGE="$(readlink -f "$SOURCE_IMAGE")"
OUTPUT_IMAGE="$(readlink -m "$OUTPUT_IMAGE")"
BUNDLE="$(readlink -f "$BUNDLE")"
[[ "$SOURCE_IMAGE" != "$OUTPUT_IMAGE" ]] || fail "source and output images must differ"
[[ ! -e "$OUTPUT_IMAGE" ]] || fail "output image already exists: $OUTPUT_IMAGE"
[[ -d "$(dirname "$OUTPUT_IMAGE")" ]] || fail "output directory does not exist: $(dirname "$OUTPUT_IMAGE")"

sudo_cmd=()
if [[ "$(id -u)" != "0" ]]; then
  command -v sudo >/dev/null 2>&1 || fail "root or sudo is required for loop mounting"
  sudo_cmd=(sudo)
fi

cleanup() {
  local rc=$?
  set +e
  if [[ -n "$MOUNT_DIR" ]] && mountpoint -q "$MOUNT_DIR"; then
    "${sudo_cmd[@]}" umount "$MOUNT_DIR"
  fi
  if [[ -n "$LOOP_DEVICE" ]]; then
    "${sudo_cmd[@]}" losetup -d "$LOOP_DEVICE"
  fi
  if [[ -n "$MOUNT_DIR" ]]; then
    rmdir "$MOUNT_DIR" 2>/dev/null || true
  fi
  exit "$rc"
}
trap cleanup EXIT INT TERM

cp --reflink=auto --sparse=always "$SOURCE_IMAGE" "$OUTPUT_IMAGE"
if (( GROW_MIB > 0 )); then
  truncate -s "+${GROW_MIB}M" "$OUTPUT_IMAGE"
fi
LOOP_DEVICE="$("${sudo_cmd[@]}" losetup --find --show --partscan "$OUTPUT_IMAGE")"

partition="$LOOP_DEVICE"
for _ in $(seq 1 50); do
  if [[ -b "${LOOP_DEVICE}p1" ]]; then
    partition="${LOOP_DEVICE}p1"
    break
  fi
  sleep 0.1
done

if (( GROW_MIB > 0 )); then
  "${sudo_cmd[@]}" losetup -c "$LOOP_DEVICE"
  if [[ "$partition" != "$LOOP_DEVICE" ]]; then
    "${sudo_cmd[@]}" parted --fix --script "$LOOP_DEVICE" \
      resizepart 1 100%
    "${sudo_cmd[@]}" partprobe "$LOOP_DEVICE"
  fi
  set +e
  "${sudo_cmd[@]}" e2fsck -fy "$partition"
  e2fsck_rc=$?
  set -e
  (( e2fsck_rc <= 1 )) \
    || fail "e2fsck failed before rootfs growth: exit=$e2fsck_rc"
  "${sudo_cmd[@]}" resize2fs "$partition"
fi

MOUNT_DIR="$(mktemp -d /tmp/actrail-kata-rootfs.XXXXXX)"
"${sudo_cmd[@]}" mount -o rw "$partition" "$MOUNT_DIR"

install_args=(
  --rootfs "$MOUNT_DIR"
  --bundle "$BUNDLE"
  --otel-endpoint "$OTEL_ENDPOINT"
  --startup-dependency "$STARTUP_DEPENDENCY"
  --socket-gid "$SOCKET_GID"
)
if [[ "$WITH_VIEWER" == "1" ]]; then
  install_args+=(--with-viewer)
fi
"${sudo_cmd[@]}" "$SCRIPT_DIR/install-rootfs.sh" "${install_args[@]}"
"${sudo_cmd[@]}" "$SCRIPT_DIR/verify-rootfs.sh" \
  --rootfs "$MOUNT_DIR" \
  --otel-endpoint "$OTEL_ENDPOINT" \
  --socket-gid "$SOCKET_GID" \
  --startup-dependency "$STARTUP_DEPENDENCY"
sync
"${sudo_cmd[@]}" umount "$MOUNT_DIR"
"${sudo_cmd[@]}" losetup -d "$LOOP_DEVICE"
LOOP_DEVICE=""
rmdir "$MOUNT_DIR"
MOUNT_DIR=""
trap - EXIT INT TERM

echo "ACTRAIL_GUEST_IMAGE_READY"
echo "source_image=$SOURCE_IMAGE"
echo "output_image=$OUTPUT_IMAGE"
echo "guest_startup_dependency=$STARTUP_DEPENDENCY"
echo "workload_socket_gid=$SOCKET_GID"
echo "image_grow_mib=$GROW_MIB"
echo "otel_endpoint_configured=true"
