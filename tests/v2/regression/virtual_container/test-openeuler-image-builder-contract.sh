#!/usr/bin/env bash
# Static contract for the reproducible, non-privileged openEuler image path.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
BUILDER="$ROOT_DIR/deploy/virtual-container/guest/build-openeuler-image.sh"
CONTAINERFILE="$ROOT_DIR/deploy/virtual-container/guest/Containerfile.openEuler"
TMPFILES_CONFIG="$ROOT_DIR/deploy/virtual-container/guest/actrail-tmpfiles.conf"
INTERFACE_DROP_IN="$ROOT_DIR/deploy/virtual-container/guest/systemd/workload-interface/kata-agent.service.d/10-actrail-workload-interface.conf"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

bash -n "$BUILDER" || fail "openEuler image builder has invalid shell syntax"
"$BUILDER" --help | grep -Fq 'mkfs.ext4 -d' \
  || fail "builder does not document the non-mount image path"
"$BUILDER" --help | grep -Fq 'root=/dev/vda1' \
  || fail "builder does not document the Kata root-device contract"
"$BUILDER" --help | grep -Fq -- '--otel-endpoint' \
  || fail "builder does not require an explicit Guest Collector endpoint"
[[ "$(grep -Fc -- '--otel-endpoint "$OTEL_ENDPOINT"' "$BUILDER")" -ge 2 ]] \
  || fail "builder must forward the endpoint to installer and verifier"
grep -Fq 'ID:-}" == "openEuler"' "$BUILDER" \
  || fail "builder does not reject a non-openEuler environment"
grep -Fq 'VERSION_ID:-}" in' "$BUILDER" \
  || fail "builder does not pin the openEuler release family"
grep -Fq -- '--installroot="$ROOTFS"' "$BUILDER" \
  || fail "builder does not create an isolated installroot"
grep -Fq 'gzip -t "$KATA_INITRD"' "$BUILDER" \
  || fail "builder does not validate the installed Kata initrd"
grep -Fq -- '--expected-agent-version' "$BUILDER" \
  || fail "builder has no Kata agent version gate"
grep -Fq -- '--require-agent-policy' "$BUILDER" \
  || fail "builder has no Kata agent-policy gate"
grep -Fq 'sbin/init' "$BUILDER" \
  || fail "builder does not support current systemd Kata initrds"
grep -Fq 'usr/lib/systemd/system/kata-agent.service' "$BUILDER" \
  || fail "builder does not extract the matching Kata agent unit"
grep -Fq 'etc/kata-opa/default-policy.rego' "$BUILDER" \
  || fail "builder does not install the matching Kata policy"
grep -Fq '[[ ! -e "$OUTPUT_IMAGE" ]]' "$BUILDER" \
  || fail "builder does not protect an existing output image"
grep -Fq 'sfdisk "$OUTPUT_IMAGE"' "$BUILDER" \
  || fail "builder does not create the first-partition disk layout"
grep -Fq 'seek=1' "$BUILDER" \
  || fail "builder does not place ext4 at the 1 MiB partition offset"
if grep -Eq '\b(losetup|mount)\b' "$BUILDER"; then
  fail "openEuler image builder unexpectedly requires loop mounting"
fi

for package in \
  clang llvm elfutils-devel musl-gcc musl-devel cpio e2fsprogs; do
  grep -Eq "^[[:space:]]+$package[[:space:]]+\\\\$" "$CONTAINERFILE" \
    || fail "builder container is missing dependency: $package"
done

grep -Fqx 'd /dev/actrail 0750 root actrail -' "$TMPFILES_CONFIG" \
  || fail "guest image does not define the early workload-interface directory"
grep -Fqx \
  'ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail' \
  "$INTERFACE_DROP_IN" \
  || fail "kata-agent does not explicitly create its /dev bind-mount source"

echo "OPENEULER_IMAGE_BUILDER_CONTRACT_OK"
