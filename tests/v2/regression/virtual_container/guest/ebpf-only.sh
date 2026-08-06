#!/bin/sh
# V2 fixture for guest eBPF collection without TLS capture.
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"

actrail_need_binaries
mount_guest_observation_fs
OPENSSL="$(openssl_bin)"
CONFIG="${ACTRAIL_CONFIG:-$ACTRAIL_BUNDLE/guest-ebpf-only.conf}"
WORK="$(prepare_workdir ebpf-only)"
PORT="${ACTRAIL_TLS_PORT:-4434}"
SERVER_PID=""

cleanup() {
  kill_pid "$SERVER_PID"
  stop_daemon "$CONFIG"
}
trap cleanup EXIT INT TERM

echo "== eBPF-only guest validation =="
echo "config=$CONFIG"
echo "kernel=$(uname -r)"
[ -e /sys/kernel/btf/vmlinux ] && echo "BTF=YES" || fail "BTF is required for eBPF-only validation"

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
  fail "actrailctl eBPF workload launch failed"
fi
cat "$WORK/launch.out"
kill_pid "$SERVER_PID"
SERVER_PID=""
sleep "$ACTRAIL_SETTLE_SECONDS"

TRACE_ID="$(first_trace_id "$CONFIG" "$WORK/launch.out")"
echo "trace=$TRACE_ID"

"$ACTRAILVIEWER" --config "$CONFIG" summary --trace-id "$TRACE_ID" >"$WORK/summary.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" events --trace-id "$TRACE_ID" >"$WORK/events.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" network --trace-id "$TRACE_ID" >"$WORK/network.out" 2>&1
"$ACTRAILVIEWER" --config "$CONFIG" payloads --trace-id "$TRACE_ID" >"$WORK/payloads.out" 2>&1 || true
cat "$WORK/summary.out"
head -40 "$WORK/events.out"
cat "$WORK/network.out"

assert_contains "$WORK/launch.out" "deployment_permissions_selected=host_ebpf:enabled,seccomp_notify:disabled"
assert_contains "$WORK/launch.out" "deployment_permissions_degraded=false"
assert_contains "$WORK/events.out" "Process"
assert_contains "$WORK/events.out" "exec"
assert_contains "$WORK/events.out" "File"
assert_contains "$WORK/network.out" "connect"
assert_not_contains "$WORK/payloads.out" "TlsUserSpace"

EVENTS="$(summary_value events <"$WORK/summary.out")"
NET_EVENTS="$(summary_value network_events <"$WORK/summary.out")"
[ "${EVENTS:-0}" -gt 0 ] || fail "expected eBPF events > 0"
[ "${NET_EVENTS:-0}" -gt 0 ] || fail "expected eBPF network_events > 0"

echo "EBPF_ONLY_OK trace=$TRACE_ID events=$EVENTS network_events=$NET_EVENTS"
