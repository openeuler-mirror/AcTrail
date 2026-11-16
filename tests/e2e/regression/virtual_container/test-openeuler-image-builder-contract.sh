#!/usr/bin/env bash
# Static contract for the clean, non-privileged openEuler 24.03 guest image path.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
BUILDER="$ROOT_DIR/deploy/virtual-container/guest/build-openeuler-image.sh"
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
grep -Fq 'ID:-}" == "openEuler"' "$BUILDER" \
  || fail "builder does not reject a non-openEuler environment"
grep -Fq '24.03*)' "$BUILDER" \
  || fail "builder does not pin openEuler 24.03"
grep -Fq -- '--installroot="$ROOTFS"' "$BUILDER" \
  || fail "builder does not create an isolated installroot"
# socat carries the VSOCK bridge in vsock-bridge egress mode. It is always
# installed so that one base image serves both egress modes; only the bridge
# unit's enablement differs per mode.
grep -Eq '^[[:space:]]+socat[[:space:]]+\\$' "$BUILDER" \
  || fail "builder does not install socat for the VSOCK egress bridge"
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
# The image stays clean: prepare-artifacts.py injects AcTrail into a copy, so
# the egress mode, endpoint and attribute mode have a single entry point.
if grep -Eq 'install-rootfs\.sh|verify-rootfs\.sh|--bundle|--otel-endpoint' "$BUILDER"; then
  fail "openEuler image builder must not install AcTrail"
fi

grep -Fqx 'd /dev/actrail 0750 root actrail -' "$TMPFILES_CONFIG" \
  || fail "guest image does not define the early workload-interface directory"
grep -Fqx \
  'ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail' \
  "$INTERFACE_DROP_IN" \
  || fail "kata-agent does not explicitly create its /dev bind-mount source"

echo "OPENEULER_IMAGE_BUILDER_CONTRACT_OK"
