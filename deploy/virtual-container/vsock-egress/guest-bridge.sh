#!/usr/bin/env bash
set -euo pipefail

LISTEN_PORT=14318
VSOCK_PORT=43180

usage() {
    cat <<'EOF'
Usage: guest-bridge.sh [--listen-port PORT] [--vsock-port PORT]

Forward Guest loopback TCP to the Host over AF_VSOCK CID 2.
EOF
}

fail() {
    printf 'guest-bridge: %s\n' "$*" >&2
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
        --listen-port)
            [[ $# -ge 2 ]] || fail '--listen-port requires a value'
            LISTEN_PORT=$2
            shift 2
            ;;
        --vsock-port)
            [[ $# -ge 2 ]] || fail '--vsock-port requires a value'
            VSOCK_PORT=$2
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

validate_port 'loopback listen port' "$LISTEN_PORT"
validate_port 'VSOCK port' "$VSOCK_PORT"
(( 10#$VSOCK_PORT >= 1027 )) \
    || fail 'VSOCK ports 1-1026 are reserved for Kata communication and debugging'

SOCAT_BIN=$(command -v socat 2>/dev/null) \
    || fail 'socat is required'

exec "$SOCAT_BIN" -d -d \
    "TCP4-LISTEN:${LISTEN_PORT},bind=127.0.0.1,reuseaddr,fork" \
    "VSOCK-CONNECT:2:${VSOCK_PORT},connect-timeout=1"
