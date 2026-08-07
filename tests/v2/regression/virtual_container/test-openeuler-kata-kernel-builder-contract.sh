#!/usr/bin/env bash
# Static contract for the openEuler 24.09 ARM64 Kata guest-kernel build path.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
BUILDER="$ROOT_DIR/deploy/virtual-container/guest/build-openeuler-kata-kernel.sh"
CONTAINERFILE="$ROOT_DIR/deploy/virtual-container/guest/Containerfile.openEuler"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

bash -n "$BUILDER" || fail "openEuler Kata kernel builder has invalid shell syntax"
"$BUILDER" --help | grep -Fq 'openEuler 24.09' \
  || fail "kernel builder does not pin the build-container release"
"$BUILDER" --help | grep -Fq 'CONFIG_VIRTIO_FS=y' \
  || fail "kernel builder does not document the required virtio-fs change"
grep -Fq 'EXPECTED_SRPM_RELEASE="4.oe2409"' "$BUILDER" \
  || fail "kernel builder does not pin the openEuler 24.09 RPM release"
grep -Fq 'EXPECTED_SRPM_SHA256=' "$BUILDER" \
  || fail "kernel builder does not authenticate the exact source RPM"
grep -Fq 'EXPECTED_BASE_CONFIG_SHA256=' "$BUILDER" \
  || fail "kernel builder does not authenticate the patched distro config"
grep -Fq 'kata_integration-openeuler.tar.gz' "$BUILDER" \
  || fail "kernel builder does not use the source RPM integration assets"
grep -Fq -- '--enable VIRTIO_FS' "$BUILDER" \
  || fail "kernel builder does not enable CONFIG_VIRTIO_FS"
grep -Fq 'ARCH=arm64 olddefconfig' "$BUILDER" \
  || fail "kernel builder does not resolve the ARM64 configuration"
grep -Fq 'ARCH=arm64 -j "$JOBS" Image' "$BUILDER" \
  || fail "kernel builder does not build the ARM64 Image target"
grep -Fq 'KBUILD_BUILD_USER="actrail"' "$BUILDER" \
  || fail "kernel builder does not stabilize Kbuild user metadata"
grep -Fq 'SOURCE_DATE_EPOCH="$source_rpm_buildtime"' "$BUILDER" \
  || fail "kernel builder does not use the source RPM build timestamp"
for expected_line in \
  CONFIG_FUSE_FS=y \
  CONFIG_VIRTIO_FS=y \
  CONFIG_VIRTIO_MMIO=y \
  CONFIG_VSOCKETS=y \
  CONFIG_VIRTIO_VSOCKETS=y \
  CONFIG_VIRTIO_VSOCKETS_COMMON=y; do
  grep -Fq "$expected_line" "$BUILDER" \
    || fail "kernel builder does not gate required capability: $expected_line"
done
grep -Fq '"$OUTPUT_KERNEL.config"' "$BUILDER" \
  || fail "kernel builder does not preserve the resolved configuration"
grep -Fq '"$OUTPUT_KERNEL.build-info"' "$BUILDER" \
  || fail "kernel builder does not preserve provenance"
if grep -Eq '(^|[[:space:]])(dnf|rpm)[[:space:]].*(install|upgrade|erase)' "$BUILDER"; then
  fail "kernel builder unexpectedly changes installed host packages"
fi

for package in \
  bc bison flex patch perl elfutils-libelf-devel dtc-devel glibc-static; do
  grep -Eq "^[[:space:]]+$package[[:space:]]+\\\\$" "$CONTAINERFILE" \
    || fail "builder container is missing kernel dependency: $package"
done

echo "OPENEULER_KATA_KERNEL_BUILDER_CONTRACT_OK"
