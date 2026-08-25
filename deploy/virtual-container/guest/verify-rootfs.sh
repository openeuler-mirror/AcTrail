#!/usr/bin/env bash
# Verify the static AcTrail installation contract in a Kata guest rootfs.
set -euo pipefail

ROOTFS=""
EXPECTED_STARTUP_DEPENDENCY="optional"
EXPECTED_SOCKET_GID=39000
EXPECTED_OTEL_ENDPOINT=""
EXPECTED_EGRESS_MODE="network"
EXPECTED_OTEL_EXPORT_ENABLED=0
EXPECTED_OTEL_ENDPOINT_CONFIGURED="false"
EXPECTED_SANDBOX_OBSERVER=0

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=otel-endpoint.sh
source "$SCRIPT_DIR/otel-endpoint.sh"

usage() {
  cat <<'EOF'
Usage: verify-rootfs.sh --rootfs DIR [--otel-endpoint URL] [--egress-mode network|vsock-bridge] [--startup-dependency optional|required] [--socket-gid GID] [--with-sandbox-observer]

This is an offline structural check. A real Kata boot and `actrailctl doctor`
are still required before declaring the guest-root startup path complete.
EOF
}

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

while [[ "$#" -gt 0 ]]; do
  case "$1" in
    --rootfs)
      [[ "$#" -ge 2 ]] || fail "--rootfs requires a value"
      ROOTFS="$2"
      shift 2
      ;;
    --startup-dependency)
      [[ "$#" -ge 2 ]] || fail "--startup-dependency requires a value"
      EXPECTED_STARTUP_DEPENDENCY="$2"
      shift 2
      ;;
    --egress-mode)
      [[ "$#" -ge 2 ]] || fail "--egress-mode requires a value"
      EXPECTED_EGRESS_MODE="$2"
      shift 2
      ;;
    --otel-endpoint)
      [[ "$#" -ge 2 ]] || fail "--otel-endpoint requires a value"
      EXPECTED_OTEL_ENDPOINT="$2"
      shift 2
      ;;
    --socket-gid)
      [[ "$#" -ge 2 ]] || fail "--socket-gid requires a value"
      [[ "$2" =~ ^[0-9]+$ ]] || fail "--socket-gid must be an integer"
      EXPECTED_SOCKET_GID="$((10#$2))"
      shift 2
      ;;
    --with-sandbox-observer)
      EXPECTED_SANDBOX_OBSERVER=1
      shift
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

[[ -n "$ROOTFS" ]] || fail "--rootfs is required"
actrail_validate_guest_otel_selection "$EXPECTED_OTEL_ENDPOINT" "$EXPECTED_EGRESS_MODE" \
  || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
if [[ -n "$EXPECTED_OTEL_ENDPOINT" ]]; then
  EXPECTED_OTEL_EXPORT_ENABLED=1
  EXPECTED_OTEL_ENDPOINT_CONFIGURED="true"
fi
[[ -d "$ROOTFS" ]] || fail "rootfs is not a directory: $ROOTFS"
case "$EXPECTED_STARTUP_DEPENDENCY" in
  optional|required) ;;
  *) fail "--startup-dependency must be optional or required" ;;
esac
(( EXPECTED_SOCKET_GID > 0 && EXPECTED_SOCKET_GID <= 2147483647 )) \
  || fail "--socket-gid must be between 1 and 2147483647"
ROOTFS="$(realpath "$ROOTFS")"
[[ "$ROOTFS" != "/" ]] || fail "refusing to verify / as an offline rootfs"

assert_file() {
  local path="$ROOTFS/${1#/}"
  [[ -f "$path" ]] || fail "installed file missing: /${1#/}"
}

assert_line() {
  local relative="$1"
  local line="$2"
  grep -Fqx -- "$line" "$ROOTFS/${relative#/}" \
    || fail "/${relative#/} is missing: $line"
}

assert_link() {
  local relative="$1"
  local expected="$2"
  local path="$ROOTFS/${relative#/}"
  [[ -L "$path" ]] || fail "enabled-unit link missing: /${relative#/}"
  [[ "$(readlink "$path")" == "$expected" ]] \
    || fail "/${relative#/} points to $(readlink "$path"), expected $expected"
}

assert_file /usr/local/bin/actraild
assert_file /usr/local/bin/actrailctl
assert_file /usr/local/lib/actrail/libactrail_tls_payload_probe_sync.so
assert_file /etc/actrail/operator.conf
assert_file /usr/lib/systemd/system/actraild.service
assert_file /usr/lib/tmpfiles.d/actrail.conf
assert_file /usr/lib/systemd/system/kata-agent.service.d/10-actrail-workload-interface.conf
assert_file /usr/share/actrail/guest-install-info
assert_file /usr/share/actrail/workload-interface
observer_agent_drop_in=/usr/lib/systemd/system/kata-agent.service.d/30-actrail-sandbox-observer.conf
[[ ! -e "$ROOTFS$observer_agent_drop_in" && ! -L "$ROOTFS$observer_agent_drop_in" ]] \
  || fail "sandbox observer must not change kata-agent startup ordering: $observer_agent_drop_in"
if [[ "$EXPECTED_SANDBOX_OBSERVER" == "1" ]]; then
  assert_file /usr/local/bin/actrail-sb
  assert_file /etc/actrail/sandbox-observer.toml
  assert_file /usr/lib/systemd/system/actrail-sb.service
  assert_file /usr/lib/systemd/system/actrail-sb-connect.service
  assert_line /etc/actrail/sandbox-observer.toml \
    'root_process_names = ["actrail-root"]'
  assert_line /etc/actrail/sandbox-observer.toml \
    'oom_event_capacity = 256'
  assert_line /etc/actrail/sandbox-observer.toml \
    'socket_path = "/dev/actrail/sandbox-observer-control.sock"'
  assert_line /usr/lib/systemd/system/actrail-sb.service \
    "ExecStart=/bin/sh -ec 'exec /usr/local/bin/actrail-sb daemon --config /etc/actrail/sandbox-observer.toml >>/dev/actrail/sandbox-observer.log 2>&1'"
  assert_line /usr/lib/systemd/system/actrail-sb.service \
    'ExecStartPost=/usr/bin/touch /dev/actrail/sandbox-observer.ready'
  assert_line /usr/lib/systemd/system/actrail-sb-connect.service \
    'ExecStart=/usr/local/bin/actrail-sb connect --control-socket /dev/actrail/sandbox-observer-control.sock --host-cid 2 --port 43182 --request-timeout-ms 5000'
  assert_link \
    /usr/lib/systemd/system/kata-containers.target.wants/actrail-sb.service \
    ../actrail-sb.service
  assert_link \
    /usr/lib/systemd/system/multi-user.target.wants/actrail-sb.service \
    ../actrail-sb.service
  for target in kata-containers.target multi-user.target; do
    connect_link="$ROOTFS/usr/lib/systemd/system/$target.wants/actrail-sb-connect.service"
    [[ ! -e "$connect_link" && ! -L "$connect_link" ]] \
      || fail "sandbox observer auto-connect must be disabled by default: $connect_link"
  done
else
  for path in \
    /usr/local/bin/actrail-sb \
    /etc/actrail/sandbox-observer.toml \
    /usr/lib/systemd/system/actrail-sb.service \
    /usr/lib/systemd/system/actrail-sb-connect.service \
    /usr/lib/systemd/system/kata-containers.target.wants/actrail-sb.service \
    /usr/lib/systemd/system/kata-containers.target.wants/actrail-sb-connect.service \
    /usr/lib/systemd/system/multi-user.target.wants/actrail-sb.service \
    /usr/lib/systemd/system/multi-user.target.wants/actrail-sb-connect.service; do
    [[ ! -e "$ROOTFS$path" && ! -L "$ROOTFS$path" ]] \
      || fail "rootfs unexpectedly contains sandbox observer path: $path"
  done
fi

assert_line /etc/actrail/operator.conf \
  'sync_runtime_library_path = "/usr/local/lib/actrail/libactrail_tls_payload_probe_sync.so"'
assert_line /etc/actrail/operator.conf \
  'socket_path = "/dev/actrail/control.sock"'
assert_line /etc/actrail/operator.conf \
  'sync_event_socket_path = "/dev/actrail/tls-sync.sock"'
assert_line /etc/actrail/operator.conf 'log_path = "/run/actrail/private/actraild.log"'
assert_line /etc/actrail/operator.conf 'path = "/run/actrail/private/actrail.sqlite"'
if [[ "$EXPECTED_OTEL_EXPORT_ENABLED" == "1" ]]; then
  assert_file /etc/actrail/plugins/otel-http/otel-http.config.toml
  assert_file /usr/share/actrail/plugins/otel-http/otel-http.plugin.toml
  assert_file /usr/share/actrail/plugins/otel-http/otel-http.config.v1.schema.json
  assert_line /etc/actrail/operator.conf \
    'manifest = "/usr/share/actrail/plugins/otel-http/otel-http.plugin.toml"'
  assert_line /etc/actrail/operator.conf \
    'plugin_config = "/etc/actrail/plugins/otel-http/otel-http.config.toml"'
  assert_line /etc/actrail/plugins/otel-http/otel-http.config.toml \
    "endpoint = \"$EXPECTED_OTEL_ENDPOINT\""
  if grep -Eiq -- 'COLLECTOR_HOST|placeholder|replace[_-]me|change[_-]me' \
    "$ROOTFS/etc/actrail/plugins/otel-http/otel-http.config.toml"; then
    fail "/etc/actrail/plugins/otel-http/otel-http.config.toml contains a placeholder"
  fi
else
  if grep -Fq -- 'kata-guest.otel-http' "$ROOTFS/etc/actrail/operator.conf"; then
    fail "local-only rootfs unexpectedly loads otel-http"
  fi
  for path in \
    /etc/actrail/plugins/otel-http/otel-http.config.toml \
    /usr/share/actrail/plugins/otel-http/otel-http.plugin.toml \
    /usr/share/actrail/plugins/otel-http/otel-http.config.v1.schema.json; do
    [[ ! -e "$ROOTFS$path" && ! -L "$ROOTFS$path" ]] \
      || fail "local-only rootfs unexpectedly contains $path"
  done
fi
for section in payload.socket seccomp_notify process_seccomp enforcement; do
  awk -v section="[$section]" '
    $0 == section { inside = 1; next }
    inside && /^\[/ { exit }
    inside && $0 == "enabled = false" { found = 1 }
    END { exit !found }
  ' "$ROOTFS/etc/actrail/operator.conf" \
    || fail "/etc/actrail/operator.conf must disable $section"
done
assert_line /usr/lib/systemd/system/actraild.service \
  'Environment=LD_LIBRARY_PATH=/usr/local/lib/actrail'
assert_line /usr/lib/systemd/system/actraild.service 'User=root'
assert_line /usr/lib/systemd/system/actraild.service 'Group=actrail'
assert_line /usr/lib/systemd/system/actraild.service 'RuntimeDirectory=actrail'
assert_line /usr/lib/systemd/system/actraild.service 'RuntimeDirectoryMode=0750'
assert_line /usr/lib/systemd/system/actraild.service \
  'ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail'
assert_line /usr/lib/systemd/system/actraild.service \
  'ExecStartPre=/usr/bin/test -d /dev/actrail'
assert_line /usr/lib/systemd/system/actraild.service \
  'WantedBy=multi-user.target kata-containers.target'
assert_line /usr/lib/tmpfiles.d/actrail.conf \
  'd /dev/actrail 0750 root actrail -'
assert_line \
  /usr/lib/systemd/system/kata-agent.service.d/10-actrail-workload-interface.conf \
  'ExecStartPre=/usr/bin/systemd-tmpfiles --create --prefix=/dev/actrail'
if grep -Fq -- 'Before=kata-agent.service' \
  "$ROOTFS/usr/lib/systemd/system/actraild.service"; then
  fail "optional actraild.service must not order itself before kata-agent"
fi
grep -Fq -- '/usr/bin/touch /run/actrail/ready' \
  "$ROOTFS/usr/lib/systemd/system/actraild.service" \
  || fail "actraild.service has no runtime readiness marker"
if grep -Fq -- 'rmdir /dev/actrail' \
  "$ROOTFS/usr/lib/systemd/system/actraild.service"; then
  fail "actraild.service must not remove the guest-wide workload interface"
fi
if grep -Fq -- '/var/' "$ROOTFS/usr/lib/systemd/system/actraild.service"; then
  fail "actraild.service writes to the read-only Kata guest rootfs"
fi

assert_link /usr/lib/systemd/system/kata-containers.target.wants/actraild.service \
  ../actraild.service
assert_link /usr/lib/systemd/system/multi-user.target.wants/actraild.service \
  ../actraild.service
assert_line /usr/share/actrail/guest-install-info \
  "guest_startup_dependency=$EXPECTED_STARTUP_DEPENDENCY"
assert_line /usr/share/actrail/guest-install-info \
  "guest_egress_mode=$EXPECTED_EGRESS_MODE"
assert_line /usr/share/actrail/guest-install-info \
  "otel_export_enabled=$EXPECTED_OTEL_EXPORT_ENABLED"
assert_line /usr/share/actrail/guest-install-info \
  "workload_socket_gid=$EXPECTED_SOCKET_GID"
assert_line /usr/share/actrail/guest-install-info \
  "sandbox_observer_installed=$EXPECTED_SANDBOX_OBSERVER"
assert_line /usr/share/actrail/workload-interface \
  "socket_gid=$EXPECTED_SOCKET_GID"
assert_line /usr/share/actrail/workload-interface \
  'guest_socket_source=/dev/actrail'
assert_line /usr/share/actrail/workload-interface \
  'workload_socket_target=/run/actrail'
grep -Eq "^actrail:x:${EXPECTED_SOCKET_GID}:$" "$ROOTFS/etc/group" \
  || fail "/etc/group does not contain the expected actrail socket group"

bridge_unit="$ROOTFS/usr/lib/systemd/system/actrail-vsock-guest-bridge.service"
bridge_script="$ROOTFS/usr/local/libexec/actrail-vsock-egress/guest-bridge.sh"
if [[ "$EXPECTED_OTEL_EXPORT_ENABLED" == "1" && "$EXPECTED_EGRESS_MODE" == "vsock-bridge" ]]; then
  [[ -f "$bridge_unit" ]] || fail "vsock-bridge rootfs is missing the Guest bridge unit"
  [[ -x "$bridge_script" ]] || fail "vsock-bridge rootfs is missing the Guest bridge script"
  [[ -x "$ROOTFS/usr/bin/socat" ]] \
    || fail "vsock-bridge rootfs is missing socat, so the bridge cannot run"
  expected_bridge_port="$(actrail_guest_otel_endpoint_port "$EXPECTED_OTEL_ENDPOINT")" \
    || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
  grep -Fqx -- \
    "ExecStart=/usr/local/libexec/actrail-vsock-egress/guest-bridge.sh --listen-port $expected_bridge_port" \
    "$bridge_unit" \
    || fail "Guest bridge unit does not listen on the endpoint port $expected_bridge_port"
  assert_link /usr/lib/systemd/system/kata-containers.target.wants/actrail-vsock-guest-bridge.service \
    ../actrail-vsock-guest-bridge.service
  assert_link /usr/lib/systemd/system/multi-user.target.wants/actrail-vsock-guest-bridge.service \
    ../actrail-vsock-guest-bridge.service
else
  [[ ! -e "$bridge_unit" && ! -L "$bridge_unit" ]] \
    || fail "network egress rootfs unexpectedly contains the Guest VSOCK bridge unit"
  [[ ! -e "$bridge_script" && ! -L "$bridge_script" ]] \
    || fail "network egress rootfs unexpectedly contains the Guest VSOCK bridge script"
fi

strict_drop_in="$ROOTFS/usr/lib/systemd/system/kata-agent.service.d/20-actrail-required.conf"
if [[ "$EXPECTED_STARTUP_DEPENDENCY" == "required" ]]; then
  [[ -f "$strict_drop_in" ]] || fail "required kata-agent dependency drop-in is missing"
  grep -Fqx -- 'Requires=actraild.service' "$strict_drop_in" \
    || fail "required dependency drop-in does not require actraild.service"
else
  [[ ! -e "$strict_drop_in" && ! -L "$strict_drop_in" ]] \
    || fail "optional dependency rootfs unexpectedly contains the required drop-in"
fi

echo "ACTRAIL_GUEST_ROOTFS_STATIC_OK"
echo "rootfs=$ROOTFS"
echo "guest_startup_dependency=$EXPECTED_STARTUP_DEPENDENCY"
echo "guest_egress_mode=$EXPECTED_EGRESS_MODE"
echo "otel_endpoint_configured=$EXPECTED_OTEL_ENDPOINT_CONFIGURED"
echo "sandbox_observer_installed=$EXPECTED_SANDBOX_OBSERVER"
