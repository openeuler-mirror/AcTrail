#!/usr/bin/env bash
# Regression contract for the V2-only daemon/workload PID namespace assertion.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
ASSERTION="$ROOT_DIR/tests/v2/regression/virtual_container/assert-pid-namespace"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-pid-namespace-test.XXXXXX")"
FAKE_ROOT="$WORK_DIR/root"

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

[[ -x "$ASSERTION" ]] || fail "PID namespace assertion is missing or not executable"
install -d "$FAKE_ROOT/bin"
cat >"$FAKE_ROOT/bin/actrailctl-private" <<'EOF'
#!/bin/sh
printf '%s\n' "$ACTRAIL_TEST_TRACE_LIST"
EOF
chmod 0755 "$FAKE_ROOT/bin/actrailctl-private"

expected_pid_namespace="$(readlink /proc/self/ns/pid)"
ACTRAIL_WORKLOAD_ROOT="$FAKE_ROOT" \
ACTRAIL_TEST_TRACE_LIST="7 namespace pid=42 pidns=$expected_pid_namespace Active/Clean" \
  "$ASSERTION" >/dev/null

set +e
ACTRAIL_WORKLOAD_ROOT="$FAKE_ROOT" \
ACTRAIL_TEST_PID_NAMESPACE="$expected_pid_namespace" \
ACTRAIL_TEST_TRACE_LIST='7 namespace pid=42 pidns=pid:[4026532250] Active/Clean' \
  "$ASSERTION" >/dev/null 2>&1
namespace_rc=$?
set -e
[[ "$namespace_rc" -ne 0 ]] \
  || fail "PID namespace assertion accepted a daemon-reported mismatch"

echo "PASS: V2 PID namespace assertion"
