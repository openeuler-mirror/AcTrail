#!/usr/bin/env bash
# Static regression test for the guest systemd startup contract.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
GUEST_UNIT="$ROOT_DIR/deploy/virtual-container/guest/actraild.service"
TMPFILES_CONFIG="$ROOT_DIR/deploy/virtual-container/guest/actrail-tmpfiles.conf"
INTERFACE_DROP_IN="$ROOT_DIR/deploy/virtual-container/guest/systemd/workload-interface/kata-agent.service.d/10-actrail-workload-interface.conf"
REQUIRED_DROP_IN="$ROOT_DIR/deploy/virtual-container/guest/systemd/required/kata-agent.service.d/20-actrail-required.conf"
LEGACY_DROP_IN="$ROOT_DIR/deploy/virtual-container/guest/systemd/fail-closed/kata-agent.service.d/20-actrail-required.conf"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_line() {
  local file="$1"
  local line="$2"
  grep -Fqx -- "$line" "$file" || fail "$file is missing: $line"
}

[[ -f "$GUEST_UNIT" ]] || fail "unit not found: $GUEST_UNIT"
[[ -f "$TMPFILES_CONFIG" ]] || fail "tmpfiles config not found: $TMPFILES_CONFIG"
[[ -f "$INTERFACE_DROP_IN" ]] \
  || fail "workload-interface drop-in not found: $INTERFACE_DROP_IN"
assert_line "$GUEST_UNIT" "After=local-fs.target"
assert_line "$GUEST_UNIT" "TimeoutStartSec=60"
assert_line "$GUEST_UNIT" "WantedBy=multi-user.target kata-containers.target"
if grep -Fq -- 'Before=kata-agent.service' "$GUEST_UNIT"; then
  fail "$GUEST_UNIT must not order the default fail-open service before kata-agent"
fi
grep -Fq -- 'actrailctl --config /etc/actrail/operator.conf doctor' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT has no active control-plane readiness gate"
if grep -Fq -- 'network-online.target' "$GUEST_UNIT"; then
  fail "$GUEST_UNIT must not wait for guest networking before kata-agent"
fi

assert_line "$GUEST_UNIT" "ExecStart=/usr/local/bin/actraild --config /etc/actrail/operator.conf run"
assert_line "$GUEST_UNIT" "Environment=LD_LIBRARY_PATH=/usr/local/lib/actrail"
assert_line "$GUEST_UNIT" \
  "ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail"
assert_line "$GUEST_UNIT" "ExecStartPre=/usr/bin/test -d /dev/actrail"
assert_line "$TMPFILES_CONFIG" "d /dev/actrail 0750 root actrail -"
assert_line "$INTERFACE_DROP_IN" \
  "ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail"
grep -Fq -- 'until /usr/local/bin/actrailctl ' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT readiness gate uses the wrong actrailctl path"
grep -Fq -- '/usr/bin/touch /run/actrail/ready' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT does not publish the runtime readiness marker"
assert_line "$GUEST_UNIT" "ExecStopPost=/usr/bin/rm -f /run/actrail/ready"
if grep -Fq -- 'rmdir /dev/actrail' "$GUEST_UNIT"; then
  fail "$GUEST_UNIT must keep the workload interface for the guest lifetime"
fi
if grep -Fq -- '/var/' "$GUEST_UNIT"; then
  fail "$GUEST_UNIT must only create writable runtime state under /run"
fi

[[ -f "$REQUIRED_DROP_IN" ]] || fail "required dependency drop-in not found: $REQUIRED_DROP_IN"
assert_line "$REQUIRED_DROP_IN" "Requires=actraild.service"
assert_line "$REQUIRED_DROP_IN" "After=actraild.service"
[[ ! -e "$LEGACY_DROP_IN" ]] \
  || fail "legacy fail-closed directory must not remain as an alias"

echo "PASS: guest systemd startup contract"
