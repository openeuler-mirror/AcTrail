#!/usr/bin/env bash
set -euo pipefail

BACKEND=''
COLLECTOR_PORT=4318
VSOCK_PORT=43180
CLH_VSOCK_SOCKET=''

usage() {
    cat <<'EOF'
Usage: host-bridge.sh --backend stratovirt|cloud-hypervisor [options]

Options:
  --collector-port PORT   Host loopback Collector port (default: 4318)
  --vsock-port PORT       AcTrail VSOCK port (default: 43180)
  --clh-vsock-socket PATH Cloud Hypervisor VM VSOCK base path
EOF
}

fail() {
    printf 'host-bridge: %s\n' "$*" >&2
    exit 2
}

validate_port() {
    local label=$1
    local value=$2
    [[ "$value" =~ ^[0-9]+$ ]] || fail "$label must be an integer"
    (( 10#$value >= 1 && 10#$value <= 65535 )) \
        || fail "$label must be between 1 and 65535"
}

while [[ $# -gt 0 ]]; do
    case "$1" in
        --backend)
            [[ $# -ge 2 ]] || fail '--backend requires a value'
            BACKEND=$2
            shift 2
            ;;
        --collector-port)
            [[ $# -ge 2 ]] || fail '--collector-port requires a value'
            COLLECTOR_PORT=$2
            shift 2
            ;;
        --vsock-port)
            [[ $# -ge 2 ]] || fail '--vsock-port requires a value'
            VSOCK_PORT=$2
            shift 2
            ;;
        --clh-vsock-socket)
            [[ $# -ge 2 ]] || fail '--clh-vsock-socket requires a value'
            CLH_VSOCK_SOCKET=$2
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

[[ "$BACKEND" == stratovirt || "$BACKEND" == cloud-hypervisor ]] \
    || fail '--backend must be stratovirt or cloud-hypervisor'
validate_port 'Collector port' "$COLLECTOR_PORT"
validate_port 'VSOCK port' "$VSOCK_PORT"
(( 10#$VSOCK_PORT >= 1027 )) \
    || fail 'VSOCK ports 1-1026 are reserved for Kata communication and debugging'

SOCAT_BIN=$(command -v socat 2>/dev/null) \
    || fail 'socat is required'

TARGET="TCP4:127.0.0.1:${COLLECTOR_PORT},connect-timeout=1"
if [[ "$BACKEND" == stratovirt ]]; then
    exec "$SOCAT_BIN" -d -d \
        "VSOCK-LISTEN:${VSOCK_PORT},fork" \
        "$TARGET"
fi

[[ -n "$CLH_VSOCK_SOCKET" ]] \
    || fail '--clh-vsock-socket is required for cloud-hypervisor'
[[ "$CLH_VSOCK_SOCKET" == /* ]] \
    || fail 'Cloud Hypervisor VSOCK base path must be absolute'
[[ "$CLH_VSOCK_SOCKET" =~ ^/[A-Za-z0-9._/-]+$ ]] \
    || fail 'Cloud Hypervisor VSOCK base path contains unsafe characters'

CLH_LISTEN_SOCKET="${CLH_VSOCK_SOCKET}_${VSOCK_PORT}"
(( ${#CLH_LISTEN_SOCKET} <= 107 )) \
    || fail 'Cloud Hypervisor VSOCK port-suffix path exceeds the UNIX socket limit'
[[ ! -e "$CLH_LISTEN_SOCKET" && ! -L "$CLH_LISTEN_SOCKET" ]] \
    || fail "Cloud Hypervisor VSOCK port-suffix path already exists: $CLH_LISTEN_SOCKET"

"$SOCAT_BIN" -d -d \
    "UNIX-LISTEN:${CLH_LISTEN_SOCKET},fork,mode=0600,unlink-close" \
    "$TARGET" &
SOCAT_PID=$!

stop_socat() {
    [[ -n "${SOCAT_PID:-}" ]] || return 0
    kill "$SOCAT_PID" 2>/dev/null || true
    wait "$SOCAT_PID" 2>/dev/null || true
    SOCAT_PID=''
}

terminate() {
    stop_socat
    exit 0
}

trap stop_socat EXIT
trap terminate HUP INT TERM

# The reconcile path remains the node-wide source of truth, but systemd work
# can lag behind a busy Kata teardown. Stop this sandbox bridge directly when
# Cloud Hypervisor removes its base socket so it cannot outlive the VM.
while kill -0 "$SOCAT_PID" 2>/dev/null; do
    if [[ ! -e "$CLH_VSOCK_SOCKET" ]]; then
        stop_socat
        exit 0
    fi
    sleep 0.1
done

if wait "$SOCAT_PID"; then
    status=0
else
    status=$?
fi
SOCAT_PID=''
exit "$status"
