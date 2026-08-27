#!/usr/bin/env bash
# Static regression test for the guest-root to workload interface contract.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
GUEST_CONFIG="$ROOT_DIR/deploy/virtual-container/guest/operator.conf"
OTEL_STARTUP="$ROOT_DIR/deploy/virtual-container/guest/otel-http-startup.toml"
GUEST_UNIT="$ROOT_DIR/deploy/virtual-container/guest/actraild.service"
SCENARIO="$ROOT_DIR/tests/v2/regression/virtual_container/v2/scenario.py"
MATRIX="$ROOT_DIR/tests/v2/regression/virtual_container/v2/matrix.py"
CONFIG="$ROOT_DIR/tests/v2/regression/virtual_container/v2/config.py"
CONTAINER_MANAGER="$ROOT_DIR/tests/v2/common/kata_runtime/container.py"
NAMESPACE_ASSERTION="$ROOT_DIR/tests/v2/regression/virtual_container/assert-pid-namespace"
INJECTOR="$ROOT_DIR/deploy/virtual-container/guest/inject-image.sh"
WORKLOAD_CONTAINERFILE="$ROOT_DIR/deploy/virtual-container/workload/Containerfile.openEuler"
XIAOO_WORKLOAD="$ROOT_DIR/tests/v2/regression/virtual_container_xiaoo_concurrency/v2/workload.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

grep -Fq 'ARG OPENCODE_VERSION=1.18.18' "$WORKLOAD_CONTAINERFILE" \
  || fail "openEuler workload does not pin the accepted OpenCode version"
grep -Fq 'useradd --uid 1000 --gid 39000' "$WORKLOAD_CONTAINERFILE" \
  || fail "openEuler workload lacks the non-root Agent passwd contract"
grep -Fq '"@opencode-ai/plugin@${OPENCODE_VERSION}"' "$WORKLOAD_CONTAINERFILE" \
  || fail "openEuler workload does not cache the matching OpenCode plugin"
grep -Fq '/opt/opencode-bootstrap-cache/models.json' "$WORKLOAD_CONTAINERFILE" \
  || fail "openEuler workload does not cache the OpenCode model catalog"
grep -Fq 'opencode/*-free' "$XIAOO_WORKLOAD" \
  || fail "OpenCode smoke does not reject non-free models"
grep -Fq 'opencode run --pure' "$XIAOO_WORKLOAD" \
  || fail "OpenCode free-model smoke is not executed inside the workload"
grep -Fq '</dev/null' "$XIAOO_WORKLOAD" \
  || fail "OpenCode smoke may block waiting for ctr exec stdin"

assert_line() {
  local file="$1"
  local line="$2"
  grep -Fqx -- "$line" "$file" || fail "$file is missing: $line"
}

assert_section_line() {
  local file="$1"
  local section="$2"
  local line="$3"
  awk -v section="[$section]" -v line="$line" '
    $0 == section { inside = 1; next }
    inside && /^\[/ { exit }
    inside && $0 == line { found = 1 }
    END { exit !found }
  ' "$file" || fail "$file section [$section] is missing: $line"
}

assert_line "$GUEST_CONFIG" 'socket_path = "/dev/actrail/control.sock"'
assert_line "$GUEST_CONFIG" 'sync_event_socket_path = "/dev/actrail/tls-sync.sock"'
assert_line "$GUEST_CONFIG" 'pid_file = "/run/actrail/private/actraild.pid"'
assert_line "$GUEST_CONFIG" 'log_path = "/run/actrail/private/actraild.log"'
assert_line "$GUEST_CONFIG" 'path = "/run/actrail/private/actrail.sqlite"'
assert_line "$GUEST_CONFIG" 'directory = "/run/actrail/private/export"'
assert_line "$OTEL_STARTUP" \
  'manifest = "/usr/share/actrail/plugins/otel-http/otel-http.plugin.toml"'
assert_line "$OTEL_STARTUP" \
  'plugin_config = "/etc/actrail/plugins/otel-http/otel-http.config.toml"'
if grep -Fq 'kata-guest.otel-http' "$GUEST_CONFIG"; then
  fail "$GUEST_CONFIG must default to local-only observation"
fi
assert_section_line "$GUEST_CONFIG" "semantic_retention.l4_payload" "enabled = true"
assert_section_line "$GUEST_CONFIG" "semantic_retention.l4_payload" "stats = true"
assert_section_line "$GUEST_CONFIG" \
  "semantic_retention.l4_payload" 'body_content = "retained"'

assert_line "$GUEST_UNIT" "User=root"
assert_line "$GUEST_UNIT" "Group=actrail"
assert_line "$GUEST_UNIT" "RuntimeDirectory=actrail"
assert_line "$GUEST_UNIT" "RuntimeDirectoryMode=0750"
grep -Fq -- 'mkdir -p /run/actrail/private' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT does not create its private runtime directory"
grep -Fq -- 'test -d /dev/actrail' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT does not verify the early workload directory"
grep -Fq -- 'systemd-tmpfiles --create --prefix=/dev/actrail' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT does not materialize its own workload directory"
grep -Fq -- 'chmod 0700 /run/actrail/private' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT does not protect private runtime state"
grep -Fq -- 'chmod 0750 /dev/actrail' "$GUEST_UNIT" \
  || fail "$GUEST_UNIT does not protect the workload socket directory"
if grep -Fq -- '/var/' "$GUEST_UNIT"; then
  fail "$GUEST_UNIT writes to the read-only Kata guest rootfs"
fi

grep -Fq -- 'KataMount(Path("/dev/actrail"), "/run/actrail", read_only=True)' \
  "$SCENARIO" \
  || fail "V2 scenario does not mount the guest socket read-only"
grep -Fq -- 'workload_socket_target=/run/actrail' \
  "$ROOT_DIR/deploy/virtual-container/workload/prepare-bundle.sh" \
  || fail "workload bundle does not use the Kata 3.32 socket target"
grep -Fq -- 'KataMount(self._workload_bundle, "/opt/actrail", read_only=True)' \
  "$SCENARIO" \
  || fail "V2 scenario does not mount workload tools read-only"
grep -Fq -- '--user' "$CONTAINER_MANAGER" \
  || fail "Kata lifecycle manager does not apply workload UID/GID"
grep -Fq -- '/usr/bin/setpriv' "$CONTAINER_MANAGER" \
  || fail "Kata lifecycle manager has no containerd 1.6 user fallback"
grep -Fq -- '"--require-ebpf"' "$SCENARIO" \
  || fail "V2 data scenario does not gate its eBPF guest kernel"
grep -Fq -- '[ -S /run/actrail/control.sock ]' "$SCENARIO" \
  || fail "V2 scenario does not wait for daemon readiness"
grep -Fq -- '-naccept 1' "$SCENARIO" \
  || fail "V2 TLS server is not bounded to one test connection"
grep -Fq -- 'name="deny"' "$MATRIX" \
  || fail "V2 matrix does not test wrong-GID access rejection"
grep -Fq -- 'name="launch"' "$MATRIX" \
  || fail "V2 matrix does not exercise a completed launch"
grep -Fq -- 'name="namespace"' "$MATRIX" \
  || fail "V2 matrix does not exercise daemon-resolved PID namespace"
grep -Fq -- '/opt/actrail-test/assert-pid-namespace' "$MATRIX" \
  || fail "V2 matrix does not assert the trace PID namespace"
grep -Fq -- 'KataMount(self._case_dir, "/opt/actrail-test", read_only=True)' \
  "$SCENARIO" \
  || fail "V2 scenario does not mount assertion tools read-only"
[[ -x "$NAMESPACE_ASSERTION" ]] \
  || fail "PID namespace assertion is not an executable V2 test tool"
grep -Fq -- '/proc/self/ns/pid' "$NAMESPACE_ASSERTION" \
  || fail "PID namespace assertion does not query the workload namespace"
grep -Fq -- 'pidns=$EXPECTED_PID_NAMESPACE ' "$NAMESPACE_ASSERTION" \
  || fail "PID namespace assertion does not query daemon trace coordinates"
if grep -Fq -- 'container=' "$NAMESPACE_ASSERTION"; then
  fail "PID namespace assertion must not use container ID as a test criterion"
fi
grep -Fq -- '/opt/actrail/bin/actrail-init' "$MATRIX" \
  || fail "V2 matrix does not use the workload PID 1 supervisor"
grep -Fq -- 'program_interpreter=' \
  "$ROOT_DIR/deploy/virtual-container/workload/prepare-bundle.sh" \
  || fail "workload bundle does not declare its actrailctl ELF interpreter"
if grep -Fq -- 'export LD_LIBRARY_PATH' \
  "$ROOT_DIR/deploy/virtual-container/workload/actrail-launch"; then
  fail "workload launcher exports its private library path to the Agent"
fi
grep -Fq -- 'docker.io/library/actrail-openeuler-workload:24.09' "$CONFIG" \
  || fail "V2 runner does not default to the openEuler workload image"
grep -Fq -- '[ \"${ID:-}\" = openEuler ]' "$SCENARIO" \
  || fail "V2 scenario does not assert the workload distribution"
grep -Fq -- 'ARG BASE_IMAGE=openeuler/openeuler:24.09' "$WORKLOAD_CONTAINERFILE" \
  || fail "workload image does not pin the openEuler release"
grep -Eq '^[[:space:]]*RUN dnf install -y .*util-linux' "$WORKLOAD_CONTAINERFILE" \
  || fail "openEuler workload image does not provide setpriv for containerd 1.6"
grep -Fq -- 'secrets.token_hex(16)' "$CONTAINER_MANAGER" \
  || fail "Kata lifecycle manager reuses a fixed containerd runtime handle"
[[ "$(grep -Fc -- '--socket-gid "$SOCKET_GID"' "$INJECTOR")" -ge 2 ]] \
  || fail "guest image injector must forward the workload socket GID to installer and verifier"
grep -Fq -- '--startup-dependency "$STARTUP_DEPENDENCY"' "$INJECTOR" \
  || fail "guest image injector does not forward the Guest startup dependency"
grep -Fq -- 'install_args+=(--otel-endpoint "$OTEL_ENDPOINT")' "$INJECTOR" \
  || fail "guest image injector does not conditionally enable OTLP export"
"$INJECTOR" --help | grep -Fq -- '--with-sandbox-observer' \
  || fail "guest image injector does not expose sandbox observer injection"
[[ "$(grep -Fc -- '--with-sandbox-observer' "$INJECTOR")" -ge 3 ]] \
  || fail "guest image injector must forward sandbox observer selection to installer and verifier"
if grep -Fq -- '--mode' "$INJECTOR"; then
  fail "guest image injector retains the deprecated startup-mode interface"
fi
if grep -Eq -- '(^|[[:space:]])--privileged([[:space:]]|$)' \
  "$SCENARIO" "$CONTAINER_MANAGER"; then
  fail "V2 workload interface must not require a privileged workload"
fi
if grep -Fq -- '/actraild' "$SCENARIO" "$MATRIX"; then
  fail "V2 workload interface must not start actraild inside the workload"
fi

echo "PASS: guest-root workload interface contract"
