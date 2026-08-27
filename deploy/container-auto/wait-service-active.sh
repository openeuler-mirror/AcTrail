#!/usr/bin/env bash
# Wait until a systemd unit reaches the exact active state.
set -euo pipefail

if [[ "$#" -ne 1 ]]; then
    echo "usage: $0 UNIT" >&2
    exit 2
fi

UNIT=$1
SYSTEMCTL=${ACTRAIL_SYSTEMCTL:-systemctl}
JOURNALCTL=${ACTRAIL_JOURNALCTL:-journalctl}
ATTEMPTS=${ACTRAIL_SERVICE_READY_ATTEMPTS:-30}
INTERVAL=${ACTRAIL_SERVICE_READY_INTERVAL:-1}

[[ "$ATTEMPTS" =~ ^[1-9][0-9]*$ ]] || {
    echo "ACTRAIL_SERVICE_READY_ATTEMPTS must be a positive integer" >&2
    exit 2
}
[[ "$INTERVAL" =~ ^[0-9]+([.][0-9]+)?$ ]] || {
    echo "ACTRAIL_SERVICE_READY_INTERVAL must be a non-negative number" >&2
    exit 2
}

attempt=1
while [[ "$attempt" -le "$ATTEMPTS" ]]; do
    state="$($SYSTEMCTL show "$UNIT" --property=ActiveState --value 2>/dev/null || true)"
    if [[ "$state" == "active" ]]; then
        exit 0
    fi
    if [[ "$attempt" -lt "$ATTEMPTS" ]]; then
        sleep "$INTERVAL"
    fi
    attempt=$((attempt + 1))
done

echo "FAIL: $UNIT did not become active (last state: ${state:-unknown})" >&2
$SYSTEMCTL status "$UNIT" --no-pager >&2 || true
$JOURNALCTL -u "$UNIT" -n 50 --no-pager >&2 || true
exit 1
