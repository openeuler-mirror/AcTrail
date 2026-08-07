#!/usr/bin/env bash
# Verify that virtual-container preflight checks only the selected VMM backend.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
PREFLIGHT="$ROOT_DIR/tests/v2/regression/virtual_container/preflight.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

run_preflight() {
  local backend="$1"
  local output_file="$2"
  set +e
  BACKEND="$backend" \
  KATA_CONFIG_DIRS="$CONFIG_DIR" \
  RUNTIME_CONFIG_PATH= \
    "$PREFLIGHT" >"$output_file" 2>&1
  set -e
}

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-preflight-backend.XXXXXX")"
trap 'rm -rf -- "$WORK_DIR"' EXIT
CONFIG_DIR="$WORK_DIR/config"
mkdir -p "$CONFIG_DIR"
touch \
  "$CONFIG_DIR/configuration-clh.toml" \
  "$CONFIG_DIR/configuration-stratovirt.toml"

run_preflight cloud-hypervisor "$WORK_DIR/cloud-hypervisor.out"
grep -Eq "Cloud Hypervisor|cloud-hypervisor" "$WORK_DIR/cloud-hypervisor.out" \
  || fail "Cloud Hypervisor preflight did not check Cloud Hypervisor"
if grep -Eq "StratoVirt|stratovirt -V" "$WORK_DIR/cloud-hypervisor.out"; then
  cat "$WORK_DIR/cloud-hypervisor.out" >&2
  fail "Cloud Hypervisor preflight unexpectedly checked StratoVirt"
fi
grep -Fq "$CONFIG_DIR/configuration-clh.toml" "$WORK_DIR/cloud-hypervisor.out" \
  || fail "Cloud Hypervisor preflight did not check its Kata runtime config"
if grep -Fq "configuration-stratovirt.toml" "$WORK_DIR/cloud-hypervisor.out"; then
  fail "Cloud Hypervisor preflight unexpectedly checked the StratoVirt config"
fi

run_preflight stratovirt "$WORK_DIR/stratovirt.out"
grep -Fq "StratoVirt" "$WORK_DIR/stratovirt.out" \
  || fail "StratoVirt preflight did not check StratoVirt"
if grep -Eq "Cloud Hypervisor|cloud-hypervisor" \
  "$WORK_DIR/stratovirt.out"; then
  cat "$WORK_DIR/stratovirt.out" >&2
  fail "StratoVirt preflight unexpectedly checked Cloud Hypervisor"
fi
grep -Fq "$CONFIG_DIR/configuration-stratovirt.toml" \
  "$WORK_DIR/stratovirt.out" \
  || fail "StratoVirt preflight did not check its Kata runtime config"
if grep -Fq "configuration-clh.toml" "$WORK_DIR/stratovirt.out"; then
  fail "StratoVirt preflight unexpectedly checked the Cloud Hypervisor config"
fi

run_preflight all "$WORK_DIR/all.out"
grep -Fq "StratoVirt" "$WORK_DIR/all.out" \
  || fail "all-backend preflight did not check StratoVirt"
grep -Eq "Cloud Hypervisor|cloud-hypervisor" "$WORK_DIR/all.out" \
  || fail "all-backend preflight did not check Cloud Hypervisor"
grep -Fq "$CONFIG_DIR/configuration-stratovirt.toml" "$WORK_DIR/all.out" \
  || fail "all-backend preflight did not check the StratoVirt config"
grep -Fq "$CONFIG_DIR/configuration-clh.toml" "$WORK_DIR/all.out" \
  || fail "all-backend preflight did not check the Cloud Hypervisor config"

set +e
BACKEND=unsupported RUNTIME_CONFIG_PATH= \
  "$PREFLIGHT" >"$WORK_DIR/unsupported.out" 2>&1
unsupported_rc=$?
set -e
[[ "$unsupported_rc" -ne 0 ]] \
  || fail "unsupported backend unexpectedly passed preflight"
grep -Fq "unsupported BACKEND=unsupported" "$WORK_DIR/unsupported.out" \
  || {
    cat "$WORK_DIR/unsupported.out" >&2
    fail "unsupported backend did not produce a clear diagnostic"
  }

echo "PREFLIGHT_BACKEND_CONTRACT_TEST_OK"
