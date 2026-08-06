#!/bin/sh
# V2 fixture for TLS capture without requiring guest eBPF.
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"

actrail_need_binaries
OPENSSL="$(openssl_bin)"
CONFIG="${ACTRAIL_CONFIG:-$ACTRAIL_BUNDLE/guest-tls-only.conf}"
WORK="$(prepare_workdir tls-only)"
PORT="${ACTRAIL_TLS_PORT:-4433}"
SERVER_PID=""

cleanup() {
  kill_pid "$SERVER_PID"
  stop_daemon "$CONFIG"
}
trap cleanup EXIT INT TERM

echo "== TLS-only guest validation =="
echo "config=$CONFIG"
echo "openssl=$OPENSSL"

generate_tls_cert "$OPENSSL" "$WORK"
start_daemon "$CONFIG"
SERVER_PID="$(start_tls_server "$OPENSSL" "$WORK" "$PORT")"
sleep 1

if ! printf "CLIENT_SECRET_MARKER_7788\n" | "$ACTRAILCTL" \
  --config "$CONFIG" \
  launch --host-ebpf auto --seccomp-notify disabled -- \
  "$OPENSSL" s_client -connect "127.0.0.1:$PORT" -quiet \
  >"$WORK/launch.out" 2>&1; then
  cat "$WORK/launch.out"
  fail "actrailctl TLS workload launch failed"
fi
cat "$WORK/launch.out"
kill_pid "$SERVER_PID"
SERVER_PID=""
sleep "$ACTRAIL_SETTLE_SECONDS"

TRACE_ID="$(first_trace_id "$CONFIG" "$WORK/launch.out")"
echo "trace=$TRACE_ID"

"$ACTRAILVIEWER" --config "$CONFIG" summary --trace-id "$TRACE_ID" >"$WORK/summary.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" payloads --trace-id "$TRACE_ID" >"$WORK/payloads.out" 2>&1
cat "$WORK/summary.out"
cat "$WORK/payloads.out"

assert_contains "$WORK/launch.out" "deployment_permissions_degraded=false"
assert_contains "$WORK/payloads.out" "TlsUserSpace"
assert_contains "$WORK/payloads.out" "SSL_write"
assert_contains "$WORK/payloads.out" "SSL_read"
assert_contains "$WORK/payloads.out" "26/26"
assert_contains "$WORK/payloads.out" "18/18"
assert_tls_payload_contents "$CONFIG" "$TRACE_ID" "$WORK/payloads.out" "$WORK"

echo "TLS_ONLY_OK trace=$TRACE_ID tls=openssl_SSL_write_SSL_read"
