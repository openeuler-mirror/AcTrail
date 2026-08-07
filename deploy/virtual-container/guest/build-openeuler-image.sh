#!/usr/bin/env bash
# Build a systemd openEuler Kata rootfs and partitioned ext4 image without loop mounts.
set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=otel-endpoint.sh
source "$SCRIPT_DIR/otel-endpoint.sh"
ROOTFS=""
OUTPUT_IMAGE=""
KATA_INITRD=""
BUNDLE=""
OTEL_ENDPOINT=""
KATA_SYSTEMD_DIR=""
EXPECTED_AGENT_VERSION=""
REQUIRE_AGENT_POLICY=0
STARTUP_DEPENDENCY="optional"
SOCKET_GID=39000
SIZE_MIB=1024
WITH_VIEWER=0
TEMP_DIR=""

usage() {
  cat <<'EOF'
Usage:
  build-openeuler-image.sh \
    --rootfs DIR \
    --output-image FILE \
    --kata-initrd FILE \
    --bundle DIR \
    --otel-endpoint URL [options]

Options:
  --otel-endpoint URL          Guest-reachable OTLP/HTTP traces URL (required)
  --kata-systemd-dir DIR       Override matching kata-agent.service and target;
                               default: extract both from the reference initrd
  --expected-agent-version V  Require `kata-agent --version` to contain V
  --require-agent-policy      Require and install default-policy.rego from initrd
  --startup-dependency POLICY optional or required (default: optional)
  --socket-gid GID            Workload interface GID (default: 39000)
  --size-mib N                First-partition ext4 size in MiB (default: 1024);
                              the partition table adds 1 MiB to the disk image
  --with-viewer               Include actrailviewer for data-plane acceptance
  -h, --help                  Show this help

Run as root inside an openEuler 24.03 build container. Both output paths must
not exist (an existing empty rootfs directory is accepted). The script uses
dnf --installroot, `mkfs.ext4 -d`, sfdisk, and dd; it never mounts a filesystem
and does not require a privileged container. The ext4 filesystem is placed in
the first partition for Kata's `root=/dev/vda1` layout.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --rootfs)
      [[ "$#" -ge 2 ]] || fail "--rootfs requires a value"
      ROOTFS="$2"
      shift 2
      ;;
    --output-image)
      [[ "$#" -ge 2 ]] || fail "--output-image requires a value"
      OUTPUT_IMAGE="$2"
      shift 2
      ;;
    --kata-initrd)
      [[ "$#" -ge 2 ]] || fail "--kata-initrd requires a value"
      KATA_INITRD="$2"
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
    --kata-systemd-dir)
      [[ "$#" -ge 2 ]] || fail "--kata-systemd-dir requires a value"
      KATA_SYSTEMD_DIR="$2"
      shift 2
      ;;
    --expected-agent-version)
      [[ "$#" -ge 2 ]] || fail "--expected-agent-version requires a value"
      EXPECTED_AGENT_VERSION="$2"
      shift 2
      ;;
    --require-agent-policy)
      REQUIRE_AGENT_POLICY=1
      shift
      ;;
    --startup-dependency)
      [[ "$#" -ge 2 ]] || fail "--startup-dependency requires a value"
      STARTUP_DEPENDENCY="$2"
      shift 2
      ;;
    --socket-gid)
      [[ "$#" -ge 2 ]] || fail "--socket-gid requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--socket-gid must be an integer"
      SOCKET_GID="$((10#$2))"
      shift 2
      ;;
    --size-mib)
      [[ "$#" -ge 2 ]] || fail "--size-mib requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--size-mib must be an integer"
      SIZE_MIB="$((10#$2))"
      shift 2
      ;;
    --with-viewer)
      WITH_VIEWER=1
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

[[ "$(id -u)" -eq 0 ]] || fail "run this script as root inside the build container"
[[ -r /etc/os-release ]] || fail "cannot identify the build-container OS"
# shellcheck disable=SC1091
. /etc/os-release
[[ "${ID:-}" == "openEuler" ]] \
  || fail "build container must be openEuler, found ID=${ID:-unknown}"
case "${VERSION_ID:-}" in
  24.03*) ;;
  *) fail "build container must be openEuler 24.03, found VERSION_ID=${VERSION_ID:-unknown}" ;;
esac

[[ -n "$ROOTFS" ]] || fail "--rootfs is required"
[[ -n "$OUTPUT_IMAGE" ]] || fail "--output-image is required"
[[ -n "$KATA_INITRD" ]] || fail "--kata-initrd is required"
[[ -n "$BUNDLE" ]] || fail "--bundle is required"
actrail_validate_guest_otel_endpoint "$OTEL_ENDPOINT" \
  || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
[[ -f "$KATA_INITRD" ]] || fail "Kata initrd not found: $KATA_INITRD"
[[ -d "$BUNDLE" ]] || fail "AcTrail guest bundle not found: $BUNDLE"
if [[ -n "$KATA_SYSTEMD_DIR" ]]; then
  [[ -f "$KATA_SYSTEMD_DIR/kata-agent.service" ]] \
    || fail "kata-agent.service not found in $KATA_SYSTEMD_DIR"
  [[ -f "$KATA_SYSTEMD_DIR/kata-containers.target" ]] \
    || fail "kata-containers.target not found in $KATA_SYSTEMD_DIR"
fi
case "$STARTUP_DEPENDENCY" in
  optional|required) ;;
  *) fail "--startup-dependency must be optional or required" ;;
esac
(( SOCKET_GID > 0 && SOCKET_GID <= 2147483647 )) \
  || fail "--socket-gid must be between 1 and 2147483647"
(( SIZE_MIB >= 512 )) || fail "--size-mib must be at least 512"

for command_name in \
  cpio dd dnf file gzip install mkfs.ext4 mktemp readlink rm sfdisk truncate; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "missing build-container command: $command_name"
done

ROOTFS="$(readlink -m "$ROOTFS")"
OUTPUT_IMAGE="$(readlink -m "$OUTPUT_IMAGE")"
KATA_INITRD="$(readlink -f "$KATA_INITRD")"
BUNDLE="$(readlink -f "$BUNDLE")"
if [[ -n "$KATA_SYSTEMD_DIR" ]]; then
  KATA_SYSTEMD_DIR="$(readlink -f "$KATA_SYSTEMD_DIR")"
fi
[[ "$ROOTFS" != "/" ]] || fail "refusing to use / as the rootfs"
[[ ! -e "$OUTPUT_IMAGE" ]] || fail "output image already exists: $OUTPUT_IMAGE"
[[ -d "$(dirname "$OUTPUT_IMAGE")" ]] \
  || fail "output image directory does not exist: $(dirname "$OUTPUT_IMAGE")"
if [[ -e "$ROOTFS" ]]; then
  [[ -d "$ROOTFS" ]] || fail "rootfs path is not a directory: $ROOTFS"
  [[ -z "$(find "$ROOTFS" -mindepth 1 -print -quit)" ]] \
    || fail "rootfs directory must be empty: $ROOTFS"
else
  install -d -m 0755 "$ROOTFS"
fi

cleanup() {
  local rc=$?
  set +e
  [[ -z "$TEMP_DIR" ]] || rm -rf -- "$TEMP_DIR"
  exit "$rc"
}
trap cleanup EXIT INT TERM

echo "== create openEuler 24.03 guest installroot =="
dnf -y \
  --installroot="$ROOTFS" \
  --releasever=24.03-LTS-SP3 \
  --setopt=install_weak_deps=False \
  --setopt=keepcache=False \
  install \
    bash \
    ca-certificates \
    chrony \
    coreutils \
    glibc \
    iproute \
    iptables \
    kmod \
    libgcc \
    libseccomp \
    openssl \
    procps-ng \
    shadow \
    systemd \
    util-linux
dnf -y --installroot="$ROOTFS" clean all
rm -rf -- "$ROOTFS/var/cache/dnf"

echo "== extract matching Kata agent assets from the reference initrd =="
gzip -t "$KATA_INITRD" \
  || fail "only a gzip-compressed Kata initrd is supported"
TEMP_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-kata-agent.XXXXXX")"
(
  cd "$TEMP_DIR"
  gzip -dc "$KATA_INITRD" | cpio -idm --quiet \
    init \
    sbin/init \
    'etc/kata-opa/*' \
    usr/lib/systemd/system/kata-agent.service \
    usr/lib/systemd/system/kata-containers.target
)

# Older minimal initrds use /init as kata-agent. Current systemd initrds use
# /init -> /sbin/init and store kata-agent itself at /sbin/init. Never follow
# an absolute /init symlink into the build container's own root filesystem.
agent_source=""
if [[ -f "$TEMP_DIR/sbin/init" && -x "$TEMP_DIR/sbin/init" ]]; then
  agent_source="$TEMP_DIR/sbin/init"
elif [[ -f "$TEMP_DIR/init" && -x "$TEMP_DIR/init" && ! -L "$TEMP_DIR/init" ]]; then
  agent_source="$TEMP_DIR/init"
fi
[[ -n "$agent_source" ]] \
  || fail "Kata initrd does not contain a recognizable agent executable"
agent_version="$("$agent_source" --version 2>&1 || true)"
[[ "$agent_version" == *"kata-agent version"* ]] \
  || fail "reference initrd executable is not a recognizable Kata agent"
if [[ -n "$EXPECTED_AGENT_VERSION" ]]; then
  [[ "$agent_version" == *"kata-agent version $EXPECTED_AGENT_VERSION"* ]] \
    || fail "Kata agent version mismatch: $agent_version"
fi
echo "$agent_version"

if [[ -n "$KATA_SYSTEMD_DIR" ]]; then
  agent_unit_source="$KATA_SYSTEMD_DIR/kata-agent.service"
  agent_target_source="$KATA_SYSTEMD_DIR/kata-containers.target"
else
  agent_unit_source="$TEMP_DIR/usr/lib/systemd/system/kata-agent.service"
  agent_target_source="$TEMP_DIR/usr/lib/systemd/system/kata-containers.target"
  [[ -f "$agent_unit_source" ]] \
    || fail "reference initrd does not contain kata-agent.service"
  [[ -f "$agent_target_source" ]] \
    || fail "reference initrd does not contain kata-containers.target"
fi

agent_policy_source="$TEMP_DIR/etc/kata-opa/default-policy.rego"
if [[ "$REQUIRE_AGENT_POLICY" == "1" && ! -e "$agent_policy_source" ]]; then
  fail "reference initrd does not contain a usable default-policy.rego"
fi

unit_dir="$ROOTFS/usr/lib/systemd/system"
install -d -m 0755 "$ROOTFS/usr/bin" "$unit_dir" "$ROOTFS/etc/systemd/system"
install -m 0755 "$agent_source" "$ROOTFS/usr/bin/kata-agent"
install -m 0644 \
  "$agent_unit_source" \
  "$unit_dir/kata-agent.service"
install -m 0644 \
  "$agent_target_source" \
  "$unit_dir/kata-containers.target"
agent_policy="absent"
if [[ -e "$agent_policy_source" ]]; then
  install -d -m 0755 "$ROOTFS/etc/kata-opa"
  # Dereference the release initrd's default-policy symlink so the generated
  # openEuler rootfs has one self-contained policy file.
  install -m 0644 "$agent_policy_source" \
    "$ROOTFS/etc/kata-opa/default-policy.rego"
  agent_policy="installed"
fi
ln -sfn /usr/lib/systemd/system/kata-containers.target \
  "$ROOTFS/etc/systemd/system/default.target"
rm -f -- "$ROOTFS/etc/machine-id"
install -m 0644 /dev/null "$ROOTFS/etc/machine-id"

echo "== inject AcTrail guest service =="
install_args=(
  --rootfs "$ROOTFS"
  --bundle "$BUNDLE"
  --otel-endpoint "$OTEL_ENDPOINT"
  --startup-dependency "$STARTUP_DEPENDENCY"
  --socket-gid "$SOCKET_GID"
)
if [[ "$WITH_VIEWER" == "1" ]]; then
  install_args+=(--with-viewer)
fi
"$SCRIPT_DIR/install-rootfs.sh" "${install_args[@]}"
"$SCRIPT_DIR/verify-rootfs.sh" \
  --rootfs "$ROOTFS" \
  --otel-endpoint "$OTEL_ENDPOINT" \
  --startup-dependency "$STARTUP_DEPENDENCY" \
  --socket-gid "$SOCKET_GID"

echo "== create partitioned sparse ext4 guest image without mounting =="
rootfs_image="$TEMP_DIR/kata-rootfs.ext4"
truncate -s "${SIZE_MIB}M" "$rootfs_image"
mkfs.ext4 -F -L kataRootfs -d "$ROOTFS" "$rootfs_image"

# Keep the runtime's standard root=/dev/vda1 command line by placing the
# filesystem at a 1 MiB aligned first partition.
truncate -s "$((SIZE_MIB + 1))M" "$OUTPUT_IMAGE"
sfdisk "$OUTPUT_IMAGE" >/dev/null <<'EOF'
label: dos
unit: sectors

start=2048, type=83
EOF
dd \
  if="$rootfs_image" \
  of="$OUTPUT_IMAGE" \
  bs=1M \
  seek=1 \
  conv=notrunc,sparse \
  status=none
chmod 0644 "$OUTPUT_IMAGE"

trap - EXIT INT TERM
rm -rf -- "$TEMP_DIR"
TEMP_DIR=""

echo "ACTRAIL_OPENEULER_GUEST_IMAGE_READY"
echo "builder_os=${PRETTY_NAME:-openEuler 24.03}"
echo "rootfs=$ROOTFS"
echo "output_image=$OUTPUT_IMAGE"
echo "rootfs_partition_size_mib=$SIZE_MIB"
echo "disk_image_size_mib=$((SIZE_MIB + 1))"
echo "root_device=/dev/vda1"
echo "agent=$agent_version"
echo "agent_policy=$agent_policy"
echo "guest_startup_dependency=$STARTUP_DEPENDENCY"
echo "workload_socket_gid=$SOCKET_GID"
echo "otel_endpoint_configured=true"
