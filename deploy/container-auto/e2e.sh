#!/usr/bin/env bash
# AcTrail Docker permission auto-selection — four-way end-to-end check.
#
# Exercises the full host-eBPF × workload-seccomp-notify matrix against one
# unified config and verifies the immutable profile selected for every trace.
set -euo pipefail

MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${MODULE_DIR}/../.." && pwd)"
BIN_DIR="${BIN_DIR:-${REPO_ROOT}/target/release}"
PROFILE="${MODULE_DIR}/seccomp/actrail-notify.json"
TARGET_URL="${TARGET_URL:-https://example.com/}"
BASE_IMAGE="${CONTAINER_AUTO_E2E_BASE_IMAGE:-openeuler/openeuler:24.03-lts-sp3}"
PROBE_LIB="libactrail_tls_payload_probe_sync.so"
CONTAINER_CONF="/etc/actrail/container-auto.conf"
RUN_ID="$(date +%s)-$$"
WORK_DIR="$(mktemp -d /tmp/actrail-container-auto-e2e.XXXXXX)"
SOCK_DIR="${WORK_DIR}/run"
DATA_DIR="${WORK_DIR}/data"
LOG_DIR="${WORK_DIR}/log"
PLUGIN_SOURCE_DIR="${REPO_ROOT}/examples/plugins/builtin/otel-jsonl"
PLUGIN_DIR="${WORK_DIR}/etc/actrail/plugins/otel-jsonl"
BUILD_CONTEXT="${WORK_DIR}/image"
CONF="${WORK_DIR}/container-auto.conf"
AUTO_CONF="${WORK_DIR}/container-auto.auto.conf"
DB="${DATA_DIR}/actrail.sqlite"
DAEMON_LOG="${LOG_DIR}/actraild.stderr"
IMAGE="actrail-agent:auto-${RUN_ID}"
IMAGE_E2E="actrail-agent:auto-e2e-${RUN_ID}"
RUN_LABEL="io.actrail.e2e-run=${RUN_ID}"
AUTO_BOTH="actrail-auto-both-${RUN_ID}"
AUTO_HOST_ONLY="actrail-auto-host-only-${RUN_ID}"
AUTO_NOTIFY_ONLY="actrail-auto-notify-only-${RUN_ID}"
AUTO_NEITHER="actrail-auto-neither-${RUN_ID}"
PEER_A="actrail-peer-a-${RUN_ID}"
PEER_B="actrail-peer-b-${RUN_ID}"
PEER_HOST_PID="actrail-peer-host-pid-${RUN_ID}"
DAEMON_PID=""

pass() { echo "  PASS: $*"; }
fail() { echo "  FAIL: $*" >&2; exit 1; }
note() { echo "==> $*"; }

stop_test_daemon() {
    local state=""
    if [[ -z "${DAEMON_PID}" ]]; then
        return
    fi
    if kill -0 "${DAEMON_PID}" >/dev/null 2>&1; then
        kill -TERM "${DAEMON_PID}" >/dev/null 2>&1 || true
        for _ in $(seq 1 50); do
            kill -0 "${DAEMON_PID}" >/dev/null 2>&1 || break
            if [[ -r "/proc/${DAEMON_PID}/stat" ]]; then
                read -r _ _ state _ <"/proc/${DAEMON_PID}/stat" || true
                [[ "${state}" == "Z" ]] && break
            fi
            sleep 0.1
        done
        if kill -0 "${DAEMON_PID}" >/dev/null 2>&1 \
            && [[ "${state:-}" != "Z" ]]; then
            kill -KILL "${DAEMON_PID}" >/dev/null 2>&1 || true
        fi
    fi
    wait "${DAEMON_PID}" >/dev/null 2>&1 || true
    DAEMON_PID=""
}

cleanup() {
    local owned_containers=()
    local owned_images=()
    stop_test_daemon
    if command -v docker >/dev/null 2>&1; then
        mapfile -t owned_containers \
            < <(docker ps -aq --filter "label=${RUN_LABEL}" 2>/dev/null || true)
        if [[ ${#owned_containers[@]} -gt 0 ]]; then
            docker rm -f "${owned_containers[@]}" >/dev/null 2>&1 || true
        fi
        mapfile -t owned_images \
            < <(docker image ls -q --filter "label=${RUN_LABEL}" 2>/dev/null || true)
        if [[ ${#owned_images[@]} -gt 0 ]]; then
            docker image rm -f "${owned_images[@]}" >/dev/null 2>&1 || true
        fi
    fi
    rm -rf "${WORK_DIR}"
}
trap cleanup EXIT

wait_for_sockets() {
    for _ in $(seq 1 50); do
        [[ -S "${SOCK_DIR}/control.sock" && -S "${SOCK_DIR}/tls-sync.sock" ]] \
            && return 0
        kill -0 "${DAEMON_PID}" >/dev/null 2>&1 || return 1
        sleep 0.2
    done
    return 1
}

start_test_daemon() {
    stop_test_daemon
    rm -f "${SOCK_DIR}/control.sock" "${SOCK_DIR}/tls-sync.sock" \
        "${SOCK_DIR}/actraild.pid"
    "${BIN_DIR}/actraild" --config "${CONF}" run >>"${DAEMON_LOG}" 2>&1 &
    DAEMON_PID=$!
    if ! wait_for_sockets; then
        tail -n 80 "${DAEMON_LOG}" >&2 || true
        fail "isolated test daemon sockets did not appear"
    fi
}

ctl() {
    "${BIN_DIR}/actrailctl" --config "${CONF}" "$@"
}

latest_trace_after() {
    local previous="$1"
    local trace=""
    for _ in $(seq 1 50); do
        trace="$(sqlite3 "${DB}" \
            "SELECT trace_id FROM traces WHERE trace_id > ${previous} ORDER BY trace_id DESC LIMIT 1;" \
            2>/dev/null || true)"
        if [[ -n "${trace}" ]]; then
            printf '%s\n' "${trace}"
            return 0
        fi
        sleep 0.2
    done
    return 1
}

run_matrix_case() {
    local name="$1"
    local expected_profile="$2"
    local expected_host="$3"
    local expected_seccomp="$4"
    local seccomp_profile="$5"
    local before trace actual_profile output payloads ebpf_events notify_events
    local security_args=()

    if [[ "${seccomp_profile}" == "custom" ]]; then
        security_args=(--security-opt "seccomp=${PROFILE}")
    fi
    before="$(sqlite3 "${DB}" 'SELECT COALESCE(MAX(trace_id), 0) FROM traces;')"
    output="$(docker run --name "${name}" --label "${RUN_LABEL}" --user 0:0 \
        "${security_args[@]}" \
        -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
        -v "${CONF}:${CONTAINER_CONF}:ro" \
        "${IMAGE_E2E}" sh -c \
        "curl -sS ${TARGET_URL} -o /dev/null && /bin/echo ${name}-ok")"

    echo "${output}" \
        | grep -q "deployment_permissions_selected=host_ebpf:${expected_host},seccomp_notify:${expected_seccomp}" \
        || fail "${name}: selected permission axes are wrong"
    trace="$(latest_trace_after "${before}")" || fail "${name}: no trace created"
    actual_profile="$(sqlite3 "${DB}" \
        "SELECT profile_name FROM traces WHERE trace_id=${trace};")"
    [[ "${actual_profile}" == "${expected_profile}" ]] \
        || fail "${name}: expected profile ${expected_profile}, got ${actual_profile}"
    payloads="$(sqlite3 "${DB}" \
        "SELECT COUNT(*) FROM payload_segments WHERE trace_id=${trace};")"
    [[ "${payloads}" -gt 0 ]] || fail "${name}: TLS plaintext was not captured"
    for _ in $(seq 1 50); do
        ebpf_events="$(sqlite3 "${DB}" \
            "SELECT COUNT(*) FROM events WHERE trace_id=${trace} AND collector='ebpf';")"
        notify_events="$(sqlite3 "${DB}" \
            "SELECT COUNT(*) FROM events WHERE trace_id=${trace} AND collector='process-seccomp';")"
        if [[ "${expected_host}" == "disabled" || "${ebpf_events}" -gt 0 ]] \
            && [[ "${expected_seccomp}" == "disabled" || "${notify_events}" -gt 0 ]]; then
            break
        fi
        sleep 0.2
    done
    if [[ "${expected_host}" == "enabled" ]]; then
        [[ "${ebpf_events}" -gt 0 ]] || fail "${name}: expected eBPF events"
    else
        [[ "${ebpf_events}" -eq 0 ]] || fail "${name}: unexpected eBPF events"
    fi
    if [[ "${expected_seccomp}" == "enabled" ]]; then
        [[ "${notify_events}" -gt 0 ]] || fail "${name}: expected process-seccomp events"
    else
        [[ "${notify_events}" -eq 0 ]] || fail "${name}: unexpected process-seccomp events"
    fi
    [[ "$(docker inspect --format '{{.HostConfig.Privileged}}' "${name}")" == "false" ]] \
        || fail "${name}: container is privileged"
    [[ -z "$(docker inspect --format '{{.HostConfig.PidMode}}' "${name}")" ]] \
        || fail "${name}: container uses host PID"
    [[ "$(docker inspect --format '{{json .HostConfig.CapAdd}}' "${name}")" == "null" ]] \
        || fail "${name}: container received extra capabilities"
    docker rm "${name}" >/dev/null
    pass "${name}: host_ebpf=${expected_host}, seccomp_notify=${expected_seccomp}"
}

note "0) preflight"
[[ ${EUID} -eq 0 ]] || fail "run as root"
command -v docker >/dev/null || fail "docker missing"
command -v sqlite3 >/dev/null || fail "sqlite3 missing"
command -v sed >/dev/null || fail "sed missing"
[[ -f "${PROFILE}" ]] || fail "missing seccomp profile"
for f in actraild actrailctl "${PROBE_LIB}"; do
    [[ -f "${BIN_DIR}/${f}" ]] || fail "missing ${BIN_DIR}/${f}"
done
for f in \
    otel-jsonl.plugin.toml \
    otel-jsonl.config.toml \
    otel-jsonl.config.v1.schema.json; do
    [[ -f "${PLUGIN_SOURCE_DIR}/${f}" ]] \
        || fail "missing ${PLUGIN_SOURCE_DIR}/${f}"
done
mkdir -p \
    "${SOCK_DIR}" \
    "${DATA_DIR}/export" \
    "${LOG_DIR}" \
    "${PLUGIN_DIR}" \
    "${BUILD_CONTEXT}"
install -m 0644 \
    "${PLUGIN_SOURCE_DIR}/otel-jsonl.plugin.toml" \
    "${PLUGIN_SOURCE_DIR}/otel-jsonl.config.v1.schema.json" \
    "${PLUGIN_DIR}/"
sed \
    -e "s|/var/lib/actrail|${DATA_DIR}|g" \
    "${PLUGIN_SOURCE_DIR}/otel-jsonl.config.toml" \
    >"${PLUGIN_DIR}/otel-jsonl.config.toml"
sed \
    -e "s|/run/actrail|${SOCK_DIR}|g" \
    -e "s|/var/lib/actrail|${DATA_DIR}|g" \
    -e "s|/var/log/actrail|${LOG_DIR}|g" \
    -e "s|/etc/actrail|${WORK_DIR}/etc/actrail|g" \
    -e "/^\[payload\.tls\]$/a sync_event_socket_path = \"${SOCK_DIR}/tls-sync.sock\"" \
    "${MODULE_DIR}/container-auto.conf" >"${CONF}"
cat >>"${CONF}" <<EOF

[control]
socket_path = "${SOCK_DIR}/control.sock"
pid_file = "${SOCK_DIR}/actraild.pid"
log_path = "${DAEMON_LOG}"

[storage]
backend = "sqlite"

[storage.sqlite]
path = "${DB}"

[export.snapshot]
directory = "${DATA_DIR}/export"
EOF
cp "${CONF}" "${AUTO_CONF}"

note "1) start isolated unified auto daemon"
grep -q "^socket_path = \"${SOCK_DIR}/control.sock\"$" "${CONF}" \
    || fail "generated auto config control socket is not isolated"
grep -q "^path = \"${DB}\"$" "${CONF}" \
    || fail "generated auto config storage is not isolated"
grep -q "^sync_event_socket_path = \"${SOCK_DIR}/tls-sync.sock\"$" "${CONF}" \
    || fail "generated auto config TLS-sync socket is not isolated"
start_test_daemon
grep -q '^profile_name = "container-auto"$' "${CONF}" \
    || fail "generated auto profile name is wrong"
grep -A1 '^\[ebpf\]$' "${CONF}" | grep -q '^enabled = "auto"$' \
    || fail "generated auto config does not use ebpf auto"
DOCTOR="$(ctl doctor)"
echo "${DOCTOR}" | grep -q 'collectors=.*ebpf' \
    || fail "test host must provide eBPF for the upper matrix row"
pass "unified config started with host eBPF available"

note "2) build auto agent image"
install -m 0755 "${BIN_DIR}/actrailctl" "${BUILD_CONTEXT}/actrailctl"
install -m 0755 "${BIN_DIR}/${PROBE_LIB}" \
    "${BUILD_CONTEXT}/${PROBE_LIB}"
install -m 0644 "${MODULE_DIR}/Dockerfile" "${BUILD_CONTEXT}/Dockerfile"
docker build -q -f "${BUILD_CONTEXT}/Dockerfile" \
    --build-arg "BASE_IMAGE=${BASE_IMAGE}" \
    --label "${RUN_LABEL}" -t "${IMAGE}" "${BUILD_CONTEXT}" >/dev/null
printf '%s\n' \
    "FROM ${IMAGE}" \
    'RUN set -eux; if command -v apt-get >/dev/null 2>&1; then apt-get update; DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends curl python3; rm -rf /var/lib/apt/lists/*; elif command -v dnf >/dev/null 2>&1; then dnf install -y curl python3; dnf clean all; rm -rf /var/cache/dnf; else echo "unsupported base image: apt-get or dnf is required" >&2; exit 1; fi' \
    | docker build -q --label "${RUN_LABEL}" -t "${IMAGE_E2E}" - >/dev/null
pass "auto image built"

note "3) host eBPF available: custom/default seccomp"
run_matrix_case \
    "${AUTO_BOTH}" container-auto-ebpf-on-notify-on enabled enabled custom
run_matrix_case \
    "${AUTO_HOST_ONLY}" container-auto-ebpf-on-notify-off enabled disabled default

note "4) disable host eBPF and exercise lower matrix row"
sed -i '/^\[ebpf\]/,/^\[/ s/^enabled = "auto"$/enabled = false/' "${CONF}"
start_test_daemon
DOCTOR="$(ctl doctor)"
echo "${DOCTOR}" | grep -q 'collectors=.*ebpf' \
    && fail "daemon still reports eBPF after it was disabled"
run_matrix_case \
    "${AUTO_NOTIFY_ONLY}" container-auto-ebpf-off-notify-on disabled enabled custom
run_matrix_case \
    "${AUTO_NEITHER}" container-auto-ebpf-off-notify-off disabled disabled default

note "5) peer authentication isolates traces across containers"
before="$(sqlite3 "${DB}" 'SELECT COALESCE(MAX(trace_id), 0) FROM traces;')"
docker run -d --name "${PEER_A}" --label "${RUN_LABEL}" --user 0:0 \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
    -v "${CONF}:${CONTAINER_CONF}:ro" \
    "${IMAGE_E2E}" /bin/sh -c 'sleep 120' >/dev/null
docker run -d --name "${PEER_B}" --label "${RUN_LABEL}" --user 0:0 \
    --entrypoint /bin/sh \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
    -v "${CONF}:${CONTAINER_CONF}:ro" \
    "${IMAGE_E2E}" -c 'sleep 120' >/dev/null
trace="$(latest_trace_after "${before}")" \
    || fail "peer isolation: container A did not create a trace"

B_LIST="$(docker exec "${PEER_B}" \
    actrailctl --config "${CONTAINER_CONF}" list-traces)"
echo "${B_LIST}" | grep -q "trace-${trace} " \
    && fail "peer isolation: container B can list container A trace"

docker run -d --name "${PEER_HOST_PID}" --label "${RUN_LABEL}" --user 0:0 \
    --pid=host \
    --entrypoint /bin/sh \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
    -v "${CONF}:${CONTAINER_CONF}:ro" \
    "${IMAGE_E2E}" -c 'sleep 120' >/dev/null
HOST_PID_LIST="$(docker exec "${PEER_HOST_PID}" \
    actrailctl --config "${CONTAINER_CONF}" list-traces)"
echo "${HOST_PID_LIST}" | grep -q "trace-${trace} " \
    && fail "peer isolation: host-PID container was trusted as host root"
if docker exec "${PEER_HOST_PID}" \
    actrailctl --config "${CONTAINER_CONF}" track-remove \
    --trace-id "trace-${trace}" >/dev/null 2>&1; then
    fail "peer isolation: host-PID container removed container A trace"
fi
pass "host-PID container does not inherit host-root authority"

if REMOVE_OUT="$(docker exec "${PEER_B}" \
    actrailctl --config "${CONTAINER_CONF}" track-remove \
    --trace-id "trace-${trace}" 2>&1)"; then
    fail "peer isolation: container B removed container A trace"
elif echo "${REMOVE_OUT}" | grep -q "peer_identity"; then
    pass "container B cannot remove container A trace"
else
    echo "${REMOVE_OUT}" | sed 's/^/    /'
    fail "peer isolation: remove rejection lacked peer_identity"
fi
missing_trace="$((trace + 1000000))"
if MISSING_REMOVE_OUT="$(docker exec "${PEER_B}" \
    actrailctl --config "${CONTAINER_CONF}" track-remove \
    --trace-id "trace-${missing_trace}" 2>&1)"; then
    fail "peer isolation: removing a missing trace unexpectedly succeeded"
fi
[[ "${REMOVE_OUT}" == "${MISSING_REMOVE_OUT}" ]] \
    || fail "peer isolation: track-remove reveals whether another trace exists"
pass "track-remove does not disclose cross-container trace existence"
[[ "$(sqlite3 "${DB}" \
    "SELECT lifecycle_state FROM traces WHERE trace_id=${trace};")" == "active" ]] \
    || fail "peer isolation: container A trace stopped after rejected remove"

REGISTER_OUT="$(docker exec "${PEER_B}" python3 -c '
import array
import os
import socket
import sys

fields = [
    b"register_seccomp_listener_v2",
    b"9001",
    sys.argv[1].encode(),
    str(os.getpid()).encode(),
    os.readlink("/proc/self/ns/pid").encode(),
]
frame = b"".join(str(len(field)).encode() + b"#" + field for field in fields)
listener_fd = os.open("/dev/null", os.O_RDONLY)
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(sys.argv[2])
client.sendmsg(
    [frame],
    [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [listener_fd]).tobytes())],
)
reply = client.recv(65536)
os.close(listener_fd)
sys.stdout.buffer.write(reply)
' "${trace}" "${SOCK_DIR}/control.sock")"
echo "${REGISTER_OUT}" | grep -q "peer_identity" \
    || fail "peer isolation: container B seccomp registration was not rejected"
pass "container B cannot register a seccomp listener for container A trace"

AUDIT_OFFSET="$(wc -c <"${DAEMON_LOG}")"
docker exec "${PEER_B}" python3 -c '
import os
import socket
import sys

client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(sys.argv[2])
# Send a valid v2 frame so the test reaches peer authorization instead of
# being rejected by the wire decoder.
stat = open("/proc/self/stat", encoding="ascii").read()
start_time_ticks = stat[stat.rfind(")") + 2 :].split()[19]
fields = [
    "v2",
    "payload",
    sys.argv[1],
    str(os.getpid()),
    start_time_ticks,
    os.readlink("/proc/self/ns/pid"),
    "outbound",
    "peer-e2e",
    "injection",
    "1",
    "1",
    "6869",
]
client.sendall(("\t".join(fields) + "\n").encode())
client.shutdown(socket.SHUT_WR)
client.settimeout(10)
try:
    while client.recv(4096):
        pass
except socket.timeout:
    pass
' "${trace}" "${SOCK_DIR}/tls-sync.sock"
tls_rejected=false
for _ in $(seq 1 50); do
    TLS_AUDIT="$(tail -c "+$((AUDIT_OFFSET + 1))" "${DAEMON_LOG}" 2>/dev/null || true)"
    if echo "${TLS_AUDIT}" | grep -q "closed rejected TLS-sync peer" \
        && echo "${TLS_AUDIT}" | grep -q "trace trace-${trace}"; then
        tls_rejected=true
        break
    fi
    sleep 0.2
done
[[ "${tls_rejected}" == "true" ]] \
    || fail "peer isolation: no audited rejection for container B TLS injection"
[[ "$(sqlite3 "${DB}" \
    "SELECT COUNT(*) FROM payload_segments WHERE trace_id=${trace} AND library='peer-e2e' AND symbol='injection';")" -eq 0 ]] \
    || fail "peer isolation: forged TLS payload reached container A trace"
pass "container B cannot inject TLS events into container A trace"
docker rm -f "${PEER_A}" "${PEER_B}" "${PEER_HOST_PID}" >/dev/null

note "6) required permissions fail loud"
if REQUIRED_OUT="$(docker run --rm --label "${RUN_LABEL}" --user 0:0 \
    --entrypoint /usr/local/bin/actrailctl \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" -v "${CONF}:${CONTAINER_CONF}:ro" \
    "${IMAGE}" --config "${CONTAINER_CONF}" launch \
    --host-ebpf required --seccomp-notify disabled -- /bin/true 2>&1)"; then
    fail "required host eBPF unexpectedly launched while collector was disabled"
elif echo "${REQUIRED_OUT}" | grep -q "host eBPF required"; then
    pass "required host eBPF fails loud"
else
    echo "${REQUIRED_OUT}" | sed 's/^/    /'
    fail "required host eBPF failure lacked a stable diagnostic"
fi

if REQUIRED_OUT="$(docker run --rm --label "${RUN_LABEL}" --user 0:0 \
    --entrypoint /usr/local/bin/actrailctl \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" -v "${CONF}:${CONTAINER_CONF}:ro" \
    "${IMAGE}" --config "${CONTAINER_CONF}" launch \
    --host-ebpf disabled --seccomp-notify required -- /bin/true 2>&1)"; then
    fail "required seccomp-notify unexpectedly launched under Docker default seccomp"
elif echo "${REQUIRED_OUT}" | grep -q "seccomp-notify required"; then
    pass "required seccomp-notify fails loud"
else
    echo "${REQUIRED_OUT}" | sed 's/^/    /'
    fail "required seccomp-notify failure lacked a stable diagnostic"
fi

note "6.5) host.id stamped into OTLP export"
# Read the export this run actually wrote (DATA_DIR), not a stale fixed path.
OTLP_JSONL="${DATA_DIR}/export/live-spans.otlp.jsonl"
HOST_UUID="$(tr -d '[:space:]' < /sys/class/dmi/id/product_uuid 2>/dev/null || true)"
if [[ -z "${HOST_UUID}" ]]; then
    echo "  SKIP: no DMI product_uuid on this machine; host.id auto-probe not exercised"
else
    for _ in $(seq 1 25); do
        [[ -f "${OTLP_JSONL}" ]] \
            && grep -q '"resourceSpans"' "${OTLP_JSONL}" 2>/dev/null && break
        sleep 0.2
    done
    [[ -f "${OTLP_JSONL}" ]] || fail "host.id: OTLP jsonl ${OTLP_JSONL} was not produced"
    # Every exported record must carry host.id — not just "some record".
    total="$(grep -c '"resourceSpans"' "${OTLP_JSONL}" 2>/dev/null || echo 0)"
    withhost="$(grep -c '"host.id"' "${OTLP_JSONL}" 2>/dev/null || echo 0)"
    [[ "${total}" -gt 0 ]] || fail "host.id: no OTLP records exported"
    [[ "${total}" -eq "${withhost}" ]] \
        || fail "host.id: only ${withhost}/${total} exported records carry host.id"
    grep -q "${HOST_UUID}" "${OTLP_JSONL}" \
        || fail "host.id: exported value does not match DMI product_uuid ${HOST_UUID}"
    pass "host.id=${HOST_UUID} on all ${total} exported OTLP records"
fi

note "7) restore eBPF auto"
cp "${AUTO_CONF}" "${CONF}"
start_test_daemon
ctl doctor | grep -q 'collectors=.*ebpf' \
    || fail "host eBPF did not recover after restoring auto config"
pass "all four auto combinations and required-permission guards passed"

echo
echo "E2E permission auto-selection: all assertions passed."
