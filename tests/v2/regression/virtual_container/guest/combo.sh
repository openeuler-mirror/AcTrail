#!/bin/sh
# V2 fixture for guest eBPF and TLS capture in the same trace.
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"

actrail_need_binaries
mount_guest_observation_fs
OPENSSL="$(openssl_bin)"
CONFIG="${ACTRAIL_CONFIG:-$ACTRAIL_BUNDLE/guest-combo.conf}"
WORK="$(prepare_workdir combo)"
PORT="${ACTRAIL_TLS_PORT:-4435}"
SERVER_PID=""

cleanup() {
  kill_pid "$SERVER_PID"
  stop_daemon "$CONFIG"
}
trap cleanup EXIT INT TERM

echo "== eBPF + TLS same-trace guest validation =="
echo "config=$CONFIG"
echo "kernel=$(uname -r)"
[ -e /sys/kernel/btf/vmlinux ] && echo "BTF=YES" || fail "BTF is required for combo validation"

generate_tls_cert "$OPENSSL" "$WORK"
start_daemon "$CONFIG"
SERVER_PID="$(start_tls_server "$OPENSSL" "$WORK" "$PORT")"
sleep 1

if ! printf "CLIENT_SECRET_MARKER_7788\n" | "$ACTRAILCTL" \
  --config "$CONFIG" \
  launch --host-ebpf required --seccomp-notify disabled -- \
  "$OPENSSL" s_client -connect "127.0.0.1:$PORT" -quiet \
  >"$WORK/launch.out" 2>&1; then
  cat "$WORK/launch.out"
  fail "actrailctl combo workload launch failed"
fi
cat "$WORK/launch.out"
kill_pid "$SERVER_PID"
SERVER_PID=""
sleep "$ACTRAIL_SETTLE_SECONDS"

TRACE_ID="$(first_trace_id "$CONFIG" "$WORK/launch.out")"
echo "trace=$TRACE_ID"

"$ACTRAILVIEWER" --config "$CONFIG" summary --trace-id "$TRACE_ID" >"$WORK/summary.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" payloads --trace-id "$TRACE_ID" >"$WORK/payloads.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" events --trace-id "$TRACE_ID" >"$WORK/events.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" network --trace-id "$TRACE_ID" >"$WORK/network.out" 2>&1
cat "$WORK/summary.out"
cat "$WORK/payloads.out"
head -40 "$WORK/events.out"
cat "$WORK/network.out"

assert_contains "$WORK/launch.out" "deployment_permissions_selected=host_ebpf:enabled,seccomp_notify:disabled"
assert_contains "$WORK/launch.out" "deployment_permissions_degraded=false"
assert_contains "$WORK/payloads.out" "TlsUserSpace"
assert_contains "$WORK/payloads.out" "SSL_write"
assert_contains "$WORK/payloads.out" "SSL_read"
assert_contains "$WORK/payloads.out" "26/26"
assert_contains "$WORK/payloads.out" "18/18"
assert_tls_payload_contents "$CONFIG" "$TRACE_ID" "$WORK/payloads.out" "$WORK"
assert_contains "$WORK/events.out" "Process"
assert_contains "$WORK/events.out" "exec"
assert_contains "$WORK/network.out" "connect"

EVENTS="$(summary_value events <"$WORK/summary.out")"
NET_EVENTS="$(summary_value network_events <"$WORK/summary.out")"
[ "${EVENTS:-0}" -gt 0 ] || fail "expected eBPF events > 0"
[ "${NET_EVENTS:-0}" -gt 0 ] || fail "expected eBPF network_events > 0"

echo "COMBO_OK trace=$TRACE_ID events=$EVENTS network_events=$NET_EVENTS tls=openssl_SSL_write_SSL_read"
