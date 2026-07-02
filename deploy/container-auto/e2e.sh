#!/usr/bin/env bash
# AcTrail Docker permission auto-selection — four-way end-to-end check.
#
# Exercises the full host-eBPF × workload-seccomp-notify matrix against one
# unified config and verifies the immutable profile selected for every trace.
set -euo pipefail

MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${MODULE_DIR}/../.." && pwd)"
BIN_DIR="${BIN_DIR:-${REPO_ROOT}/target/release}"
CONF="/etc/actrail/container-auto.conf"
PROFILE="${MODULE_DIR}/seccomp/actrail-notify.json"
DB="/var/lib/actrail/actrail.sqlite"
SOCK_DIR="/run/actrail"
IMAGE="actrail-agent:auto"
IMAGE_E2E="actrail-agent:auto-e2e"
TARGET_URL="${TARGET_URL:-https://example.com/}"
PROBE_LIB="libactrail_tls_payload_probe_sync.so"

pass() { echo "  PASS: $*"; }
fail() { echo "  FAIL: $*" >&2; exit 1; }
note() { echo "==> $*"; }

cleanup() {
    docker rm -f \
        actrail-auto-both \
        actrail-auto-host-only \
        actrail-auto-notify-only \
        actrail-auto-neither \
        actrail-peer-a \
        actrail-peer-b \
        >/dev/null 2>&1 || true
    if [[ -f "${CONF}.e2ebak" ]]; then
        cp "${CONF}.e2ebak" "${CONF}"
        rm -f "${CONF}.e2ebak"
        systemctl restart actraild >/dev/null 2>&1 || true
    fi
}
trap cleanup EXIT

wait_for_sockets() {
    for _ in $(seq 1 50); do
        [[ -S "${SOCK_DIR}/control.sock" && -S "${SOCK_DIR}/tls-sync.sock" ]] \
            && return 0
        sleep 0.2
    done
    return 1
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
    output="$(docker run --name "${name}" --user 0:0 \
        "${security_args[@]}" \
        -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
        -v "${CONF}:${CONF}:ro" \
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
command -v journalctl >/dev/null || fail "journalctl missing"
[[ -f "${PROFILE}" ]] || fail "missing seccomp profile"
for f in actraild actrailctl actrailviewer "${PROBE_LIB}"; do
    [[ -f "${BIN_DIR}/${f}" ]] || fail "missing ${BIN_DIR}/${f}"
done

note "1) install unified auto daemon"
"${MODULE_DIR}/install-host.sh" "${BIN_DIR}"
wait_for_sockets || fail "auto daemon sockets did not appear"
grep -q '^profile_name = "container-auto"$' "${CONF}" \
    || fail "generated auto profile name is wrong"
grep -A1 '^\[ebpf\]$' "${CONF}" | grep -q '^enabled = "auto"$' \
    || fail "generated auto config does not use ebpf auto"
DOCTOR="$(actrailctl --config "${CONF}" doctor)"
echo "${DOCTOR}" | grep -q 'collectors=.*ebpf' \
    || fail "test host must provide eBPF for the upper matrix row"
pass "unified config started with host eBPF available"

note "2) build auto agent image"
cp "${BIN_DIR}/actrailctl" "${BIN_DIR}/${PROBE_LIB}" "${MODULE_DIR}/"
docker build -q -f "${MODULE_DIR}/Dockerfile" \
    -t "${IMAGE}" "${MODULE_DIR}" >/dev/null
printf 'FROM %s\nRUN apt-get update && apt-get install -y --no-install-recommends curl python3 && rm -rf /var/lib/apt/lists/*\n' \
    "${IMAGE}" | docker build -q -t "${IMAGE_E2E}" - >/dev/null
pass "auto image built"

note "3) host eBPF available: custom/default seccomp"
run_matrix_case \
    actrail-auto-both container-auto-ebpf-on-notify-on enabled enabled custom
run_matrix_case \
    actrail-auto-host-only container-auto-ebpf-on-notify-off enabled disabled default

note "4) disable host eBPF and exercise lower matrix row"
cp "${CONF}" "${CONF}.e2ebak"
sed -i '/^\[ebpf\]/,/^\[/ s/^enabled = "auto"$/enabled = false/' "${CONF}"
systemctl restart actraild
wait_for_sockets || fail "daemon sockets missing after disabling eBPF"
DOCTOR="$(actrailctl --config "${CONF}" doctor)"
echo "${DOCTOR}" | grep -q 'collectors=.*ebpf' \
    && fail "daemon still reports eBPF after it was disabled"
run_matrix_case \
    actrail-auto-notify-only container-auto-ebpf-off-notify-on disabled enabled custom
run_matrix_case \
    actrail-auto-neither container-auto-ebpf-off-notify-off disabled disabled default

note "5) peer authentication isolates traces across containers"
before="$(sqlite3 "${DB}" 'SELECT COALESCE(MAX(trace_id), 0) FROM traces;')"
docker run -d --name actrail-peer-a --user 0:0 \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
    -v "${CONF}:${CONF}:ro" \
    "${IMAGE_E2E}" /bin/sh -c 'sleep 120' >/dev/null
docker run -d --name actrail-peer-b --user 0:0 \
    --entrypoint /bin/sh \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" \
    -v "${CONF}:${CONF}:ro" \
    "${IMAGE_E2E}" -c 'sleep 120' >/dev/null
trace="$(latest_trace_after "${before}")" \
    || fail "peer isolation: container A did not create a trace"

B_LIST="$(docker exec actrail-peer-b \
    actrailctl --config "${CONF}" list-traces)"
echo "${B_LIST}" | grep -q "trace-${trace} " \
    && fail "peer isolation: container B can list container A trace"

if REMOVE_OUT="$(docker exec actrail-peer-b \
    actrailctl --config "${CONF}" track-remove \
    --trace-id "trace-${trace}" 2>&1)"; then
    fail "peer isolation: container B removed container A trace"
elif echo "${REMOVE_OUT}" | grep -q "peer_identity"; then
    pass "container B cannot remove container A trace"
else
    echo "${REMOVE_OUT}" | sed 's/^/    /'
    fail "peer isolation: remove rejection lacked peer_identity"
fi
[[ "$(sqlite3 "${DB}" \
    "SELECT lifecycle_state FROM traces WHERE trace_id=${trace};")" == "active" ]] \
    || fail "peer isolation: container A trace stopped after rejected remove"

REGISTER_OUT="$(docker exec actrail-peer-b python3 -c '
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
client.connect("/run/actrail/control.sock")
client.sendmsg(
    [frame],
    [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [listener_fd]).tobytes())],
)
reply = client.recv(65536)
os.close(listener_fd)
sys.stdout.buffer.write(reply)
' "${trace}")"
echo "${REGISTER_OUT}" | grep -q "peer_identity" \
    || fail "peer isolation: container B seccomp registration was not rejected"
pass "container B cannot register a seccomp listener for container A trace"

AUDIT_SINCE="$(date '+%Y-%m-%d %H:%M:%S')"
docker exec actrail-peer-b python3 -c '
import os
import socket
import sys

client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect("/run/actrail/tls-sync.sock")
line = (
    "v1\tpayload\t"
    + sys.argv[1]
    + "\t"
    + str(os.getpid())
    + "\toutbound\tpeer-e2e\tinjection\t1\t1\t6869\n"
)
client.sendall(line.encode())
' "${trace}"
tls_rejected=false
for _ in $(seq 1 50); do
    TLS_AUDIT="$(journalctl -u actraild.service \
        --since "${AUDIT_SINCE}" --no-pager 2>/dev/null || true)"
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
docker rm -f actrail-peer-a actrail-peer-b >/dev/null

note "6) required permissions fail loud"
if REQUIRED_OUT="$(docker run --rm --user 0:0 \
    --entrypoint /usr/local/bin/actrailctl \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" -v "${CONF}:${CONF}:ro" \
    "${IMAGE}" --config "${CONF}" launch \
    --host-ebpf required --seccomp-notify disabled -- /bin/true 2>&1)"; then
    fail "required host eBPF unexpectedly launched while collector was disabled"
elif echo "${REQUIRED_OUT}" | grep -q "host eBPF required"; then
    pass "required host eBPF fails loud"
else
    echo "${REQUIRED_OUT}" | sed 's/^/    /'
    fail "required host eBPF failure lacked a stable diagnostic"
fi

if REQUIRED_OUT="$(docker run --rm --user 0:0 \
    --entrypoint /usr/local/bin/actrailctl \
    -v "${SOCK_DIR}:${SOCK_DIR}:ro" -v "${CONF}:${CONF}:ro" \
    "${IMAGE}" --config "${CONF}" launch \
    --host-ebpf disabled --seccomp-notify required -- /bin/true 2>&1)"; then
    fail "required seccomp-notify unexpectedly launched under Docker default seccomp"
elif echo "${REQUIRED_OUT}" | grep -q "seccomp-notify required"; then
    pass "required seccomp-notify fails loud"
else
    echo "${REQUIRED_OUT}" | sed 's/^/    /'
    fail "required seccomp-notify failure lacked a stable diagnostic"
fi

note "7) restore eBPF auto"
cp "${CONF}.e2ebak" "${CONF}"
rm -f "${CONF}.e2ebak"
systemctl restart actraild
wait_for_sockets || fail "daemon sockets missing after restoring auto config"
actrailctl --config "${CONF}" doctor | grep -q 'collectors=.*ebpf' \
    || fail "host eBPF did not recover after restoring auto config"
pass "all four auto combinations and required-permission guards passed"

echo
echo "E2E permission auto-selection: all assertions passed."
