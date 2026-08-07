#!/usr/bin/env bash
# Build an ARM64 Kata guest kernel from the exact openEuler 24.09 source RPM.
set -euo pipefail

EXPECTED_SRPM_NAME="kata-containers"
EXPECTED_SRPM_VERSION="3.2.0"
EXPECTED_SRPM_RELEASE="4.oe2409"
EXPECTED_SRPM_SHA256="261269ab04a524d6c5e34473cf03c82588780dbcb01536bfc7b637de8925bba0"
EXPECTED_BASE_CONFIG_SHA256="427dd39c1b232f99bee66c23c3463058c4d48d425dcbed1f9a2a7d26805dc85b"

SOURCE_RPM=""
OUTPUT_KERNEL=""
WORK_DIR=""
JOBS=""
AUTO_WORK_DIR=0
BUILD_SUCCEEDED=0

usage() {
  cat <<'EOF'
Usage:
  build-openeuler-kata-kernel.sh \
    --source-rpm kata-containers-3.2.0-4.oe2409.src.rpm \
    --output-kernel FILE [options]

Options:
  --work-dir DIR  New or empty build directory. It is kept after the build.
                  Without this option, a temporary directory is removed after
                  success and preserved after failure.
  --jobs N        Parallel make jobs (default: nproc)
  -h, --help      Show this help

Run natively on ARM64 inside an openEuler 24.09 build container. The script
verifies the exact signed openEuler source RPM, applies its kata_integration
patch series, copies config-kata-arm64, and adds only CONFIG_VIRTIO_FS=y before
running `make olddefconfig` and building arch/arm64/boot/Image.

The output is a candidate Kata guest kernel. It does not install an RPM and
does not replace the host's /var/lib/kata/kernel. Alongside the kernel it writes
FILE.config and FILE.build-info for capability and provenance checks.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --source-rpm)
      [[ "$#" -ge 2 ]] || fail "--source-rpm requires a value"
      SOURCE_RPM="$2"
      shift 2
      ;;
    --output-kernel)
      [[ "$#" -ge 2 ]] || fail "--output-kernel requires a value"
      OUTPUT_KERNEL="$2"
      shift 2
      ;;
    --work-dir)
      [[ "$#" -ge 2 ]] || fail "--work-dir requires a value"
      WORK_DIR="$2"
      shift 2
      ;;
    --jobs)
      [[ "$#" -ge 2 ]] || fail "--jobs requires a value"
      [[ "$2" =~ ^[1-9][0-9]*$ ]] || fail "--jobs must be a positive integer"
      JOBS="$((10#$2))"
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

[[ -n "$SOURCE_RPM" ]] || fail "--source-rpm is required"
[[ -n "$OUTPUT_KERNEL" ]] || fail "--output-kernel is required"
[[ -r /etc/os-release ]] || fail "cannot identify the build-container OS"
# shellcheck disable=SC1091
. /etc/os-release
[[ "${ID:-}" == "openEuler" ]] \
  || fail "build container must be openEuler, found ID=${ID:-unknown}"
case "${VERSION_ID:-}" in
  24.09*) ;;
  *) fail "kernel build container must be openEuler 24.09, found VERSION_ID=${VERSION_ID:-unknown}" ;;
esac
[[ "$(uname -m)" == "aarch64" ]] \
  || fail "native ARM64 build required, found architecture $(uname -m)"

for command_name in \
  awk bc bison cpio file flex gcc git install make mktemp nproc patch \
  readlink rm rpm rpm2cpio sha256sum tar; do
  command -v "$command_name" >/dev/null 2>&1 \
    || fail "missing build-container command: $command_name"
done

SOURCE_RPM="$(readlink -f "$SOURCE_RPM")"
OUTPUT_KERNEL="$(readlink -m "$OUTPUT_KERNEL")"
[[ -f "$SOURCE_RPM" ]] || fail "source RPM not found: $SOURCE_RPM"
[[ -d "$(dirname "$OUTPUT_KERNEL")" ]] \
  || fail "output directory does not exist: $(dirname "$OUTPUT_KERNEL")"
for output_path in \
  "$OUTPUT_KERNEL" "$OUTPUT_KERNEL.config" "$OUTPUT_KERNEL.build-info"; do
  [[ ! -e "$output_path" ]] || fail "refusing to overwrite output: $output_path"
done

if [[ -z "$JOBS" ]]; then
  JOBS="$(nproc)"
fi

if [[ -n "$WORK_DIR" ]]; then
  WORK_DIR="$(readlink -m "$WORK_DIR")"
  if [[ -e "$WORK_DIR" ]]; then
    [[ -d "$WORK_DIR" ]] || fail "work path is not a directory: $WORK_DIR"
    [[ -z "$(find "$WORK_DIR" -mindepth 1 -print -quit)" ]] \
      || fail "work directory must be empty: $WORK_DIR"
  else
    install -d -m 0755 "$WORK_DIR"
  fi
else
  WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-oe2409-kata-kernel.XXXXXX")"
  AUTO_WORK_DIR=1
fi

cleanup() {
  local rc=$?
  set +e
  if [[ "$AUTO_WORK_DIR" == "1" ]]; then
    if [[ "$BUILD_SUCCEEDED" == "1" ]]; then
      rm -rf -- "$WORK_DIR"
    else
      echo "Build work directory preserved after failure: $WORK_DIR" >&2
    fi
  fi
  trap - EXIT INT TERM
  exit "$rc"
}
trap cleanup EXIT INT TERM

echo "== verify exact openEuler 24.09 Kata source RPM =="
actual_srpm_sha256="$(sha256sum "$SOURCE_RPM" | awk '{print $1}')"
[[ "$actual_srpm_sha256" == "$EXPECTED_SRPM_SHA256" ]] \
  || fail "source RPM SHA256 mismatch: $actual_srpm_sha256"
rpm -K "$SOURCE_RPM"
actual_name="$(rpm -qp --qf '%{NAME}' "$SOURCE_RPM")"
actual_version="$(rpm -qp --qf '%{VERSION}' "$SOURCE_RPM")"
actual_release="$(rpm -qp --qf '%{RELEASE}' "$SOURCE_RPM")"
source_rpm_buildtime="$(rpm -qp --qf '%{BUILDTIME}' "$SOURCE_RPM")"
[[ "$actual_name" == "$EXPECTED_SRPM_NAME" ]] \
  || fail "source RPM name mismatch: $actual_name"
[[ "$actual_version" == "$EXPECTED_SRPM_VERSION" ]] \
  || fail "source RPM version mismatch: $actual_version"
[[ "$actual_release" == "$EXPECTED_SRPM_RELEASE" ]] \
  || fail "source RPM release mismatch: $actual_release"
[[ "$source_rpm_buildtime" =~ ^[0-9]+$ ]] \
  || fail "source RPM has a non-numeric build time: $source_rpm_buildtime"

# Keep Kbuild metadata stable even when the caller uses a numeric UID that has
# no passwd entry inside the isolated builder container.
export KBUILD_BUILD_USER="actrail"
export KBUILD_BUILD_HOST="openeuler-24.09"
export SOURCE_DATE_EPOCH="$source_rpm_buildtime"

srpm_dir="$WORK_DIR/srpm"
integration_package_dir="$WORK_DIR/kata-integration-package"
integration_source_dir="$WORK_DIR/kata-integration-source"
kernel_source_parent="$WORK_DIR/kernel-source"
install -d -m 0755 \
  "$srpm_dir" \
  "$integration_package_dir" \
  "$integration_source_dir" \
  "$kernel_source_parent"

echo "== extract kernel and integration sources =="
(
  cd "$srpm_dir"
  rpm2cpio "$SOURCE_RPM" \
    | cpio -idm --quiet --no-absolute-filenames \
        kernel.tar.gz kata_integration-openeuler.tar.gz
)
[[ -f "$srpm_dir/kernel.tar.gz" ]] \
  || fail "source RPM does not contain kernel.tar.gz"
[[ -f "$srpm_dir/kata_integration-openeuler.tar.gz" ]] \
  || fail "source RPM does not contain kata_integration-openeuler.tar.gz"
tar -xzf "$srpm_dir/kata_integration-openeuler.tar.gz" \
  -C "$integration_package_dir"
integration_archive="$integration_package_dir/kata_integration-v1.0.0.tar.gz"
[[ -f "$integration_archive" ]] \
  || fail "integration package does not contain kata_integration-v1.0.0.tar.gz"
tar -xzf "$integration_archive" -C "$integration_source_dir"

series_file="$integration_package_dir/series.conf"
[[ -f "$series_file" ]] || fail "integration package has no series.conf"
while read -r patch_name _; do
  [[ -n "$patch_name" ]] || continue
  [[ "$patch_name" != \#* ]] || continue
  patch_path="$integration_package_dir/patches/$patch_name"
  [[ -f "$patch_path" ]] || fail "integration patch not found: $patch_name"
  echo "apply integration patch: $patch_name"
  patch --batch --fuzz=0 -d "$integration_source_dir" -p1 < "$patch_path"
done < "$series_file"

base_config="$integration_source_dir/hack/config-kata-arm64"
[[ -f "$base_config" ]] || fail "patched config-kata-arm64 was not produced"
actual_base_config_sha256="$(sha256sum "$base_config" | awk '{print $1}')"
[[ "$actual_base_config_sha256" == "$EXPECTED_BASE_CONFIG_SHA256" ]] \
  || fail "patched openEuler ARM64 config SHA256 mismatch: $actual_base_config_sha256"
if grep -Fqx 'CONFIG_VIRTIO_FS=y' "$base_config"; then
  fail "base openEuler 24.09 config unexpectedly already enables CONFIG_VIRTIO_FS"
fi

tar -xzf "$srpm_dir/kernel.tar.gz" -C "$kernel_source_parent"
kernel_tree="$kernel_source_parent/kernel"
[[ -f "$kernel_tree/Makefile" && -f "$kernel_tree/Kconfig" ]] \
  || fail "kernel.tar.gz has an unexpected directory layout"
[[ -x "$kernel_tree/scripts/config" ]] \
  || fail "kernel source has no executable scripts/config"

echo "== enable the missing virtio-fs guest capability =="
install -m 0644 "$base_config" "$kernel_tree/.config"
"$kernel_tree/scripts/config" \
  --file "$kernel_tree/.config" \
  --enable VIRTIO_FS

echo "== resolve and verify ARM64 kernel configuration =="
make -C "$kernel_tree" ARCH=arm64 olddefconfig
required_config=(
  CONFIG_FUSE_FS=y
  CONFIG_VIRTIO_FS=y
  CONFIG_VIRTIO_MMIO=y
  CONFIG_VSOCKETS=y
  CONFIG_VIRTIO_VSOCKETS=y
  CONFIG_VIRTIO_VSOCKETS_COMMON=y
)
for expected_line in "${required_config[@]}"; do
  grep -Fqx "$expected_line" "$kernel_tree/.config" \
    || fail "resolved kernel config is missing: $expected_line"
done

echo "== build ARM64 Kata guest Image (jobs=$JOBS) =="
make -C "$kernel_tree" ARCH=arm64 -j "$JOBS" Image
built_kernel="$kernel_tree/arch/arm64/boot/Image"
[[ -s "$built_kernel" ]] || fail "kernel build did not produce $built_kernel"
file "$built_kernel"

kernel_release="$(make -s -C "$kernel_tree" ARCH=arm64 kernelrelease)"
final_config_sha256="$(sha256sum "$kernel_tree/.config" | awk '{print $1}')"
install -m 0755 "$built_kernel" "$OUTPUT_KERNEL"
install -m 0644 "$kernel_tree/.config" "$OUTPUT_KERNEL.config"
output_kernel_sha256="$(sha256sum "$OUTPUT_KERNEL" | awk '{print $1}')"
compiler_version="$(gcc --version | awk 'NR == 1 { print; exit }')"
{
  printf 'source_rpm=%s\n' "$SOURCE_RPM"
  printf 'source_rpm_nevr=%s-%s-%s\n' \
    "$actual_name" "$actual_version" "$actual_release"
  printf 'source_rpm_sha256=%s\n' "$actual_srpm_sha256"
  printf 'source_date_epoch=%s\n' "$source_rpm_buildtime"
  printf 'base_config_sha256=%s\n' "$actual_base_config_sha256"
  printf 'final_config_sha256=%s\n' "$final_config_sha256"
  printf 'kernel_release=%s\n' "$kernel_release"
  printf 'kernel_sha256=%s\n' "$output_kernel_sha256"
  printf 'builder_os=%s\n' "${PRETTY_NAME:-openEuler 24.09}"
  printf 'builder_arch=%s\n' "$(uname -m)"
  printf 'compiler=%s\n' "$compiler_version"
  printf 'make_jobs=%s\n' "$JOBS"
  printf '%s\n' "${required_config[@]}"
} > "$OUTPUT_KERNEL.build-info"
chmod 0644 "$OUTPUT_KERNEL.build-info"

BUILD_SUCCEEDED=1
echo "ACTRAIL_OPENEULER_KATA_KERNEL_READY"
echo "kernel=$OUTPUT_KERNEL"
echo "kernel_release=$kernel_release"
echo "kernel_sha256=$output_kernel_sha256"
echo "config=$OUTPUT_KERNEL.config"
echo "build_info=$OUTPUT_KERNEL.build-info"
echo "work_dir=$WORK_DIR"
