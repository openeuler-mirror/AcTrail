#!/usr/bin/env bash
# Verify that virtual-container preflight accepts complete cgroup v1 and v2 hosts.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
PREFLIGHT="$ROOT_DIR/tests/v2/regression/virtual_container/preflight.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

run_preflight() {
  local cgroup_root="$1"
  local output_file="$2"
  set +e
  CGROUP_ROOT="$cgroup_root" \
  BACKEND=stratovirt \
  KATA_CONFIG_DIRS="$CONFIG_DIR" \
  RUNTIME_CONFIG_PATH= \
    "$PREFLIGHT" >"$output_file" 2>&1
  set -e
}

WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-preflight-cgroup.XXXXXX")"
trap 'rm -rf -- "$WORK_DIR"' EXIT
CONFIG_DIR="$WORK_DIR/config"
V1_ROOT="$WORK_DIR/cgroup-v1"
V2_ROOT="$WORK_DIR/cgroup-v2"
INVALID_ROOT="$WORK_DIR/cgroup-invalid"
mkdir -p "$CONFIG_DIR" "$V1_ROOT" "$V2_ROOT" "$INVALID_ROOT"
touch "$CONFIG_DIR/configuration-stratovirt.toml"

for controller in \
  blkio cpu,cpuacct cpuset devices freezer hugetlb memory pids; do
  mkdir -p "$V1_ROOT/$controller"
done
touch "$V2_ROOT/cgroup.controllers"

run_preflight "$V1_ROOT" "$WORK_DIR/v1.out"
grep -Fq "PASS: 宿主 cgroup v1 必要控制器" "$WORK_DIR/v1.out" || {
  cat "$WORK_DIR/v1.out" >&2
  fail "complete cgroup v1 hierarchy was not accepted"
}

run_preflight "$V2_ROOT" "$WORK_DIR/v2.out"
grep -Fq "PASS: 宿主 cgroup v2" "$WORK_DIR/v2.out" || {
  cat "$WORK_DIR/v2.out" >&2
  fail "cgroup v2 hierarchy was not accepted"
}

mkdir -p "$INVALID_ROOT/memory"
run_preflight "$INVALID_ROOT" "$WORK_DIR/invalid.out"
grep -Fq "FAIL: 宿主 cgroup v1/v2" "$WORK_DIR/invalid.out" || {
  cat "$WORK_DIR/invalid.out" >&2
  fail "incomplete cgroup hierarchy did not fail closed"
}
grep -Fq "devices" "$WORK_DIR/invalid.out" \
  || fail "incomplete cgroup v1 diagnostic did not list missing controllers"

echo "PREFLIGHT_CGROUP_CONTRACT_TEST_OK"
