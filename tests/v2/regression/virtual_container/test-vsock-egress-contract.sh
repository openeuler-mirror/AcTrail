#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
POC_DIR="$ROOT_DIR/deploy/virtual-container/vsock-egress"
WORK_DIR="${ACTRAIL_TEST_WORK_DIR:-${TMPDIR:-/tmp}}"
mkdir -p -- "$WORK_DIR"
TEST_ROOT=$(mktemp -d "$WORK_DIR/actrail-vsock-egress-contract.XXXXXX")
TEST_BIN="$TEST_ROOT/bin"
ARGS_LOG="$TEST_ROOT/socat.args"
GUEST_OUT="$TEST_ROOT/guest.out"
HOST_OUT="$TEST_ROOT/host.out"
CLH_DIR="$TEST_ROOT/clh"
CLH_BASE="$CLH_DIR/clh.sock"
BRIDGE_PID=''

cleanup() {
    if [[ -n "$BRIDGE_PID" ]] && kill -0 "$BRIDGE_PID" 2>/dev/null; then
        kill "$BRIDGE_PID" 2>/dev/null || true
        wait "$BRIDGE_PID" 2>/dev/null || true
    fi
}
trap cleanup EXIT

fail() {
    printf 'FAIL: %s\n' "$*" >&2
    exit 1
}

assert_file_contains() {
    local file=$1
    local expected=$2
    grep -Fq -- "$expected" "$file" \
        || fail "$file does not contain: $expected"
}

assert_file_not_contains() {
    local file=$1
    local unexpected=$2
    if grep -Fq -- "$unexpected" "$file"; then
        fail "$file unexpectedly contains: $unexpected"
    fi
}

mkdir -- "$TEST_BIN"
ln -s -- "$POC_DIR/fake-socat.py" "$TEST_BIN/socat"
export PATH="$TEST_BIN:/usr/bin:/bin"
export FAKE_SOCAT_ARGS_LOG="$ARGS_LOG"

printf 'contract: Guest bridge carries loopback TCP over host VSOCK\n'
"$POC_DIR/guest-bridge.sh" >"$GUEST_OUT"
assert_file_contains "$ARGS_LOG" \
    $'-d\t-d\tTCP4-LISTEN:14318,bind=127.0.0.1,reuseaddr,fork\tVSOCK-CONNECT:2:43180,connect-timeout=1'

printf 'contract: StratoVirt bridge forwards Host VSOCK to loopback Collector\n'
"$POC_DIR/host-bridge.sh" --backend stratovirt >"$HOST_OUT"
assert_file_contains "$ARGS_LOG" \
    $'-d\t-d\tVSOCK-LISTEN:43180,fork\tTCP4:127.0.0.1:4318,connect-timeout=1'

printf 'contract: Cloud Hypervisor bridge uses only its VM port-suffix UDS\n'
mkdir -- "$CLH_DIR"
: >"$CLH_BASE"
"$POC_DIR/host-bridge.sh" \
    --backend cloud-hypervisor \
    --clh-vsock-socket "$CLH_BASE" >"$HOST_OUT"
assert_file_contains "$ARGS_LOG" \
    "UNIX-LISTEN:${CLH_BASE}_43180,fork,mode=0600,unlink-close"

printf 'contract: Cloud Hypervisor bridge refuses an existing suffix without removing it\n'
: >"${CLH_BASE}_43180"
if "$POC_DIR/host-bridge.sh" \
    --backend cloud-hypervisor \
    --clh-vsock-socket "$CLH_BASE" >"$HOST_OUT" 2>&1; then
    fail 'Cloud Hypervisor bridge accepted an existing port-suffix path'
fi
[[ -f "${CLH_BASE}_43180" ]] \
    || fail 'Cloud Hypervisor bridge removed an existing port-suffix path'
rm -f -- "${CLH_BASE}_43180"

printf 'contract: Cloud Hypervisor bridge exits with its VM socket\n'
FAKE_SOCAT_HOLD=1 "$POC_DIR/host-bridge.sh" \
    --backend cloud-hypervisor \
    --clh-vsock-socket "$CLH_BASE" >"$HOST_OUT" 2>&1 &
BRIDGE_PID=$!
for _ in {1..40}; do
    grep -Fq -- "UNIX-LISTEN:${CLH_BASE}_43180" "$ARGS_LOG" && break
    sleep 0.05
done
grep -Fq -- "UNIX-LISTEN:${CLH_BASE}_43180" "$ARGS_LOG" \
    || fail 'Cloud Hypervisor bridge did not start its socat listener'
rm -f -- "$CLH_BASE"
for _ in {1..40}; do
    kill -0 "$BRIDGE_PID" 2>/dev/null || break
    sleep 0.05
done
if kill -0 "$BRIDGE_PID" 2>/dev/null; then
    fail 'Cloud Hypervisor bridge remained alive after its VM socket disappeared'
fi
wait "$BRIDGE_PID" \
    || fail 'Cloud Hypervisor bridge did not exit cleanly with its VM socket'
BRIDGE_PID=''

printf 'contract: Guest systemd unit supervises the foreground bridge\n'
GUEST_UNIT="$POC_DIR/systemd/actrail-vsock-guest-bridge.service"
assert_file_contains "$GUEST_UNIT" 'Before=actraild.service'
assert_file_contains "$GUEST_UNIT" \
    'ExecStart=/usr/local/libexec/actrail-vsock-egress/guest-bridge.sh'
assert_file_contains "$GUEST_UNIT" 'Restart=on-failure'
assert_file_contains "$GUEST_UNIT" 'KillMode=control-group'
assert_file_contains "$GUEST_UNIT" 'MemoryMax=64M'
assert_file_contains "$GUEST_UNIT" 'TasksMax=32'
assert_file_contains "$GUEST_UNIT" 'LimitNOFILE=128'
assert_file_contains "$GUEST_UNIT" \
    'WantedBy=multi-user.target kata-containers.target'

printf 'contract: StratoVirt Host unit supervises the node listener\n'
STRATOVIRT_UNIT="$POC_DIR/systemd/actrail-vsock-host-stratovirt.service"
assert_file_contains "$STRATOVIRT_UNIT" \
    'ExecStart=/usr/local/libexec/actrail-vsock-egress/host-bridge.sh --backend stratovirt'
assert_file_contains "$STRATOVIRT_UNIT" 'Restart=on-failure'
assert_file_contains "$STRATOVIRT_UNIT" 'KillMode=control-group'
assert_file_contains "$STRATOVIRT_UNIT" 'RestrictAddressFamilies=AF_INET AF_VSOCK'
assert_file_contains "$STRATOVIRT_UNIT" 'MemoryMax=64M'
assert_file_contains "$STRATOVIRT_UNIT" 'TasksMax=32'
assert_file_contains "$STRATOVIRT_UNIT" 'LimitNOFILE=128'
assert_file_contains "$STRATOVIRT_UNIT" 'WantedBy=multi-user.target'

printf 'contract: Cloud Hypervisor Host template derives its VM base path\n'
CLH_UNIT="$POC_DIR/systemd/actrail-vsock-host-cloud-hypervisor@.service"
# Kata names the sandbox directory after the sandbox id, so the instance name is
# the only input the template needs. Hand-written per-sandbox drop-ins cannot
# keep up with sandboxes that appear and disappear on their own.
assert_file_contains "$CLH_UNIT" \
    'ExecStart=/usr/local/libexec/actrail-vsock-egress/host-bridge.sh --backend cloud-hypervisor --clh-vsock-socket /run/vc/vm/%I/clh.sock'
assert_file_not_contains "$CLH_UNIT" 'EnvironmentFile'
assert_file_contains "$CLH_UNIT" 'Restart=on-failure'
assert_file_contains "$CLH_UNIT" 'KillMode=control-group'
assert_file_contains "$CLH_UNIT" 'RestrictAddressFamilies=AF_INET AF_UNIX'
assert_file_contains "$CLH_UNIT" 'ProtectSystem=full'
assert_file_not_contains "$CLH_UNIT" 'PrivateTmp=true'
assert_file_contains "$CLH_UNIT" 'MemoryMax=64M'
assert_file_contains "$CLH_UNIT" 'TasksMax=32'
assert_file_contains "$CLH_UNIT" 'LimitNOFILE=128'

printf 'contract: Cloud Hypervisor bridges follow the sandbox lifecycle\n'
RECONCILE="$POC_DIR/ch-reconcile.sh"
RECONCILE_PATH_UNIT="$POC_DIR/systemd/actrail-vsock-host-cloud-hypervisor-reconcile.path"
RECONCILE_UNIT="$POC_DIR/systemd/actrail-vsock-host-cloud-hypervisor-reconcile.service"
[[ -x "$RECONCILE" ]] || fail "reconcile script is missing or not executable: $RECONCILE"
assert_file_contains "$RECONCILE_PATH_UNIT" 'PathModified=/run/vc/vm'
assert_file_contains "$RECONCILE_PATH_UNIT" \
    'Unit=actrail-vsock-host-cloud-hypervisor-reconcile.service'
assert_file_contains "$RECONCILE_UNIT" 'Type=oneshot'
assert_file_contains "$RECONCILE_UNIT" \
    'ExecStart=/usr/local/libexec/actrail-vsock-egress/ch-reconcile.sh'

# A live sandbox gets a bridge; an instance whose sandbox is gone is stopped.
# The production template is statically checked above with the real
# /run/vc/vm/<sandbox-id>/clh.sock path. Keep these fake IDs short so this
# contract stays below AF_UNIX's 107-byte path limit inside the V2 workspace.
VM_ROOT_PARENT="$TEST_ROOT/av"
VM_ROOT="$VM_ROOT_PARENT/vm"
SYSTEMCTL_LOG="$TEST_ROOT/systemctl.args"
ACTIVE_UNITS="$TEST_ROOT/active.units"
LIVE_SANDBOX=6f1c2b7d
DEAD_SANDBOX=00112233
mkdir -p "$VM_ROOT/$LIVE_SANDBOX"
python3 -c 'import socket,sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])' "$VM_ROOT/$LIVE_SANDBOX/clh.sock"
printf '%s\n' \
    "actrail-vsock-host-cloud-hypervisor@${DEAD_SANDBOX}.service" >"$ACTIVE_UNITS"
cat >"$TEST_BIN/systemctl" <<'FAKE_SYSTEMCTL'
#!/usr/bin/env bash
printf '%s\n' "$*" >>"$SYSTEMCTL_LOG"
if [[ "${1-}" == "list-units" ]]; then
    cat "$ACTIVE_UNITS"
fi
exit 0
FAKE_SYSTEMCTL
chmod +x "$TEST_BIN/systemctl"
: >"$SYSTEMCTL_LOG"
ACTRAIL_VSOCK_VM_ROOT="$VM_ROOT" SYSTEMCTL_LOG="$SYSTEMCTL_LOG" \
    ACTIVE_UNITS="$ACTIVE_UNITS" "$RECONCILE" \
    || fail "reconcile failed against a live sandbox"
assert_file_contains "$SYSTEMCTL_LOG" \
    "start actrail-vsock-host-cloud-hypervisor@${LIVE_SANDBOX}.service"
assert_file_contains "$SYSTEMCTL_LOG" \
    "stop actrail-vsock-host-cloud-hypervisor@${DEAD_SANDBOX}.service"
assert_file_not_contains "$SYSTEMCTL_LOG" \
    "stop actrail-vsock-host-cloud-hypervisor@${LIVE_SANDBOX}.service"

# A sandbox directory without a CH base socket is not a Cloud Hypervisor sandbox.
NO_CLH_SANDBOX=ffeeddccbbaa99887766554433221100ffeeddccbbaa99887766554433221100
mkdir -p "$VM_ROOT/$NO_CLH_SANDBOX"
: >"$SYSTEMCTL_LOG"
ACTRAIL_VSOCK_VM_ROOT="$VM_ROOT" SYSTEMCTL_LOG="$SYSTEMCTL_LOG" \
    ACTIVE_UNITS="$ACTIVE_UNITS" "$RECONCILE" \
    || fail "reconcile failed with a non-CH sandbox present"
assert_file_not_contains "$SYSTEMCTL_LOG" "$NO_CLH_SANDBOX"

# A sandbox id systemd would reinterpret is escaped, so %I restores it verbatim.
ODD_SANDBOX="odd-sandbox"
mkdir -p "$VM_ROOT/$ODD_SANDBOX"
python3 -c 'import socket,sys
s = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
s.bind(sys.argv[1])' "$VM_ROOT/$ODD_SANDBOX/clh.sock"
: >"$SYSTEMCTL_LOG"
ACTRAIL_VSOCK_VM_ROOT="$VM_ROOT" SYSTEMCTL_LOG="$SYSTEMCTL_LOG" \
    ACTIVE_UNITS="$ACTIVE_UNITS" "$RECONCILE" \
    || fail "reconcile failed on a sandbox id needing systemd escaping"
assert_file_contains "$SYSTEMCTL_LOG" \
    'start actrail-vsock-host-cloud-hypervisor@odd\x2dsandbox.service'
rm -f -- "$VM_ROOT/$ODD_SANDBOX/clh.sock"
rmdir -- "$VM_ROOT/$ODD_SANDBOX"

# A missing VM root is normal before the first sandbox starts.
: >"$SYSTEMCTL_LOG"
ACTRAIL_VSOCK_VM_ROOT="$TEST_ROOT/absent" SYSTEMCTL_LOG="$SYSTEMCTL_LOG" \
    ACTIVE_UNITS="$ACTIVE_UNITS" "$RECONCILE" \
    || fail "reconcile must tolerate a missing VM root"

printf 'contract: Collector example terminates TLS on Host loopback only\n'
COLLECTOR_CONFIG="$POC_DIR/collector/otel-collector-tls.yaml"
assert_file_contains "$COLLECTOR_CONFIG" 'endpoint: 127.0.0.1:4318'
assert_file_contains "$COLLECTOR_CONFIG" \
    'cert_file: ${env:ACTRAIL_OTELCOL_TLS_CERT}'
assert_file_contains "$COLLECTOR_CONFIG" \
    'key_file: ${env:ACTRAIL_OTELCOL_TLS_KEY}'
assert_file_not_contains "$COLLECTOR_CONFIG" 'endpoint: 0.0.0.0:4318'

printf 'contract: PASS\n'
