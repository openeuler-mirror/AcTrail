#!/usr/bin/env bash
# Build and assemble the files mounted into a Kata guest for validation.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
BUNDLE_DIR="${BUNDLE_DIR:-$ROOT_DIR/.actrail-guest-bundle}"
BUNDLE_SYSROOT="${BUNDLE_SYSROOT:-/}"
BUNDLE_TARGET_GLIBC="${BUNDLE_TARGET_GLIBC:-}"
BUILD_PROFILE="${BUILD_PROFILE:-release}"
ACTRAIL_BUILD="${ACTRAIL_BUILD:-1}"
COPY_OPENSSL="${COPY_OPENSSL:-1}"
EBPF_TRANSPORT="${EBPF_TRANSPORT:-auto}"
BUNDLE_OPENSSL="${BUNDLE_OPENSSL:-}"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

for command_name in find install mktemp readelf readlink realpath sha256sum; do
  command -v "$command_name" >/dev/null 2>&1 || fail "missing command: $command_name"
done

case "$EBPF_TRANSPORT" in
  auto|perf-buffer|ring-buffer) ;;
  *) fail "EBPF_TRANSPORT must be auto, perf-buffer, or ring-buffer; got $EBPF_TRANSPORT" ;;
esac
case "$ACTRAIL_BUILD" in
  0|1) ;;
  *) fail "ACTRAIL_BUILD must be 0 or 1; got $ACTRAIL_BUILD" ;;
esac
case "$COPY_OPENSSL" in
  0|1) ;;
  *) fail "COPY_OPENSSL must be 0 or 1; got $COPY_OPENSSL" ;;
esac

[[ -d "$BUNDLE_SYSROOT" ]] || fail "BUNDLE_SYSROOT is not a directory: $BUNDLE_SYSROOT"
BUNDLE_SYSROOT="$(readlink -f "$BUNDLE_SYSROOT")"

FINAL_BUNDLE_DIR="$(realpath -ms "$BUNDLE_DIR")"
case "$FINAL_BUNDLE_DIR" in
  /|'') fail "unsafe BUNDLE_DIR: $FINAL_BUNDLE_DIR" ;;
esac
[[ ! -L "$FINAL_BUNDLE_DIR" ]] || fail "BUNDLE_DIR must not be a symbolic link: $FINAL_BUNDLE_DIR"

elf_machine() {
  LC_ALL=C readelf -h "$1" 2>/dev/null \
    | awk -F: '/^[[:space:]]*Machine:/ {
        sub(/^[[:space:]]+/, "", $2)
        print $2
        exit
      }'
}

elf_needed() {
  LC_ALL=C readelf -d "$1" 2>/dev/null \
    | sed -n 's/.*Shared library: \[\([^]]*\)\].*/\1/p'
}

elf_interpreter() {
  LC_ALL=C readelf -l "$1" 2>/dev/null \
    | sed -n 's/.*Requesting program interpreter: \([^]]*\).*/\1/p'
}

elf_max_glibc() {
  LC_ALL=C readelf --version-info "$1" 2>/dev/null \
    | grep -o 'GLIBC_[0-9][0-9.]*' \
    | sed 's/^GLIBC_//' \
    | sort -Vu \
    | tail -1
}

version_gt() {
  local left="$1"
  local right="$2"
  [[ "$left" != "$right" ]] \
    && [[ "$(printf '%s\n%s\n' "$left" "$right" | sort -V | tail -1)" == "$left" ]]
}

resolve_sysroot_path() {
  local current="$1"
  local link=""
  local count=0

  current="$(realpath -ms "$current")"
  if [[ "$BUNDLE_SYSROOT" != "/" && "$current" != "$BUNDLE_SYSROOT"/* ]]; then
    return 1
  fi
  while [[ -L "$current" ]]; do
    count=$((count + 1))
    [[ "$count" -le 40 ]] || return 1
    link="$(readlink "$current")"
    if [[ "$link" == /* ]]; then
      current="$(realpath -ms "$BUNDLE_SYSROOT$link")"
    else
      current="$(realpath -ms "$(dirname "$current")/$link")"
    fi
    if [[ "$BUNDLE_SYSROOT" != "/" && "$current" != "$BUNDLE_SYSROOT"/* ]]; then
      return 1
    fi
  done
  [[ -e "$current" ]] || return 1
  printf '%s\n' "$current"
}

LIBRARY_SEARCH_ROOTS=()
for relative_path in lib lib64 usr/lib usr/lib64; do
  candidate_root="$BUNDLE_SYSROOT/$relative_path"
  resolved_root="$(resolve_sysroot_path "$candidate_root" || true)"
  if [[ -n "$resolved_root" && -d "$resolved_root" ]]; then
    LIBRARY_SEARCH_ROOTS+=("$resolved_root")
  fi
done
[[ "${#LIBRARY_SEARCH_ROOTS[@]}" -gt 0 ]] \
  || fail "no library search directories found under BUNDLE_SYSROOT=$BUNDLE_SYSROOT"

find_library() {
  local soname="$1"
  local machine="$2"
  local candidate=""
  local resolved=""
  local candidate_machine=""

  while IFS= read -r candidate; do
    resolved="$(resolve_sysroot_path "$candidate" || true)"
    [[ -n "$resolved" ]] || continue
    candidate_machine="$(elf_machine "$resolved")"
    if [[ "$candidate_machine" == "$machine" ]]; then
      printf '%s\n' "$resolved"
      return 0
    fi
  done < <(
    find "${LIBRARY_SEARCH_ROOTS[@]}" \
      \( -type f -o -type l \) -name "$soname" -print 2>/dev/null \
      | LC_ALL=C sort -u
  )
  return 1
}

is_base_runtime_library() {
  case "$1" in
    ld-linux*.so*|ld64.so*|libc.so.*|libdl.so.*|libgcc_s.so.*|libm.so.*|\
libpthread.so.*|libresolv.so.*|librt.so.*|libutil.so.*|linux-vdso.so.*)
      return 0
      ;;
    *)
      return 1
      ;;
  esac
}

validate_elf() {
  local path="$1"
  local expected_machine="$2"
  local label="$3"
  local actual_machine=""
  local required_glibc=""

  actual_machine="$(elf_machine "$path")"
  [[ -n "$actual_machine" ]] || fail "$label is not a readable ELF file: $path"
  [[ "$actual_machine" == "$expected_machine" ]] \
    || fail "$label architecture mismatch: expected '$expected_machine', got '$actual_machine' ($path)"

  required_glibc="$(elf_max_glibc "$path" || true)"
  if [[ -n "$required_glibc" && -n "$TARGET_GLIBC_MAX" ]] \
    && version_gt "$required_glibc" "$TARGET_GLIBC_MAX"; then
    fail "$label requires GLIBC_$required_glibc but target limit is GLIBC_$TARGET_GLIBC_MAX ($path)"
  fi
}

find_openssl() {
  local candidate=""
  local executable_root=""
  local resolved_root=""
  if [[ -n "$BUNDLE_OPENSSL" ]]; then
    [[ -f "$BUNDLE_OPENSSL" ]] || fail "BUNDLE_OPENSSL does not exist: $BUNDLE_OPENSSL"
    printf '%s\n' "$BUNDLE_OPENSSL"
    return 0
  fi
  if [[ "$BUNDLE_SYSROOT" == "/" ]] && command -v openssl >/dev/null 2>&1; then
    command -v openssl
    return 0
  fi
  for executable_root in usr/bin bin; do
    resolved_root="$(resolve_sysroot_path "$BUNDLE_SYSROOT/$executable_root" || true)"
    [[ -n "$resolved_root" && -d "$resolved_root" ]] || continue
    candidate="$resolved_root/openssl"
    if [[ -e "$candidate" ]]; then
      resolve_sysroot_path "$candidate"
      return
    fi
  done
  return 1
}

case "$BUILD_PROFILE" in
  debug)
    CARGO_PROFILE_ARGS=()
    TARGET_SUBDIR="debug"
    ;;
  release)
    CARGO_PROFILE_ARGS=(--release)
    TARGET_SUBDIR="release"
    ;;
  *)
    CARGO_PROFILE_ARGS=(--profile "$BUILD_PROFILE")
    TARGET_SUBDIR="$BUILD_PROFILE"
    ;;
esac

TARGET_BASE="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"
TARGET_DIR="${ACTRAIL_BIN_DIR:-$TARGET_BASE/$TARGET_SUBDIR}"

echo "== prepare AcTrail guest bundle =="
echo "root=$ROOT_DIR"
echo "bundle=$FINAL_BUNDLE_DIR"
echo "profile=$BUILD_PROFILE"
echo "sysroot=$BUNDLE_SYSROOT"
echo "ebpf_transport=$EBPF_TRANSPORT"

if [[ "$ACTRAIL_BUILD" == "1" ]]; then
  ACTRAIL_EBPF_EVENT_TRANSPORT="$EBPF_TRANSPORT" \
    cargo build "${CARGO_PROFILE_ARGS[@]}" \
    -p daemon \
    -p ctl \
    -p view \
    -p tls_payload_probe_sync
fi

for artifact in \
  "$TARGET_DIR/actraild" \
  "$TARGET_DIR/actrailctl" \
  "$TARGET_DIR/actrailviewer" \
  "$TARGET_DIR/libactrail_tls_payload_probe_sync.so"; do
  [[ -f "$artifact" ]] || fail "build artifact missing: $artifact"
done

TARGET_MACHINE="$(elf_machine "$TARGET_DIR/actraild")"
[[ -n "$TARGET_MACHINE" ]] || fail "cannot determine target architecture from $TARGET_DIR/actraild"
PROGRAM_INTERPRETER="$(elf_interpreter "$TARGET_DIR/actrailctl")"
case "$PROGRAM_INTERPRETER" in
  /*) ;;
  *) fail "cannot determine an absolute ELF interpreter for $TARGET_DIR/actrailctl" ;;
esac
interpreter_source="$(
  resolve_sysroot_path "$BUNDLE_SYSROOT$PROGRAM_INTERPRETER" || true
)"
[[ -n "$interpreter_source" && -x "$interpreter_source" ]] \
  || fail "actrailctl ELF interpreter is unavailable in target sysroot: $PROGRAM_INTERPRETER"

if [[ -n "$BUNDLE_TARGET_GLIBC" ]]; then
  [[ "$BUNDLE_TARGET_GLIBC" =~ ^[0-9]+(\.[0-9]+)*$ ]] \
    || fail "BUNDLE_TARGET_GLIBC must be a numeric version such as 2.34"
  TARGET_GLIBC_MAX="$BUNDLE_TARGET_GLIBC"
else
  target_libc="$(find_library libc.so.6 "$TARGET_MACHINE" || true)"
  [[ -n "$target_libc" ]] \
    || fail "cannot find target libc.so.6 for '$TARGET_MACHINE' under BUNDLE_SYSROOT=$BUNDLE_SYSROOT"
  TARGET_GLIBC_MAX="$(elf_max_glibc "$target_libc" || true)"
  [[ -n "$TARGET_GLIBC_MAX" ]] \
    || fail "cannot determine target GLIBC version from $target_libc"
fi
validate_elf "$interpreter_source" "$TARGET_MACHINE" "actrailctl ELF interpreter"

echo "target_machine=$TARGET_MACHINE"
echo "target_glibc_max=$TARGET_GLIBC_MAX"
echo "program_interpreter=$PROGRAM_INTERPRETER"

bundle_parent="$(dirname "$FINAL_BUNDLE_DIR")"
mkdir -p "$bundle_parent"
STAGING_DIR="$(mktemp -d "$bundle_parent/.actrail-guest-bundle.XXXXXX")"
# The published bundle is mounted directly into unprivileged workloads.  Keep
# mktemp's private parent behavior during setup, but make the bundle root
# traversable before files are assembled and atomically published.
chmod 0755 "$STAGING_DIR"
cleanup_staging() {
  [[ -n "${STAGING_DIR:-}" && -d "$STAGING_DIR" ]] && rm -rf "$STAGING_DIR"
}
trap cleanup_staging EXIT

BUNDLE_DIR="$STAGING_DIR"
install -d \
  "$BUNDLE_DIR/lib" \
  "$BUNDLE_DIR/plugins/otel-http" \
  "$BUNDLE_DIR/tests/guest"
install -m 0755 "$TARGET_DIR/actraild" "$BUNDLE_DIR/actraild"
install -m 0755 "$TARGET_DIR/actrailctl" "$BUNDLE_DIR/actrailctl"
install -m 0755 "$TARGET_DIR/actrailviewer" "$BUNDLE_DIR/actrailviewer"
install -m 0755 "$TARGET_DIR/libactrail_tls_payload_probe_sync.so" \
  "$BUNDLE_DIR/libactrail_tls_payload_probe_sync.so"

ELF_INPUTS=(
  "$BUNDLE_DIR/actraild"
  "$BUNDLE_DIR/actrailctl"
  "$BUNDLE_DIR/actrailviewer"
  "$BUNDLE_DIR/libactrail_tls_payload_probe_sync.so"
)

for artifact in "${ELF_INPUTS[@]}"; do
  validate_elf "$artifact" "$TARGET_MACHINE" "bundle artifact"
done

install -m 0755 "$ROOT_DIR/tests/v2/regression/virtual_container/guest/common.sh" \
  "$BUNDLE_DIR/tests/guest/common.sh"
install -m 0755 "$ROOT_DIR/tests/v2/regression/virtual_container/guest/precheck.sh" \
  "$BUNDLE_DIR/tests/guest/precheck.sh"
install -m 0755 "$ROOT_DIR/tests/v2/regression/virtual_container/guest/tls-only.sh" \
  "$BUNDLE_DIR/tests/guest/tls-only.sh"
install -m 0755 "$ROOT_DIR/tests/v2/regression/virtual_container/guest/ebpf-only.sh" \
  "$BUNDLE_DIR/tests/guest/ebpf-only.sh"
install -m 0755 "$ROOT_DIR/tests/v2/regression/virtual_container/guest/combo.sh" \
  "$BUNDLE_DIR/tests/guest/combo.sh"
for plugin_file in \
  otel-http.plugin.toml \
  otel-http.config.toml \
  otel-http.config.v1.schema.json; do
  install -m 0644 \
    "$ROOT_DIR/examples/plugins/builtin/otel-http/$plugin_file" \
    "$BUNDLE_DIR/plugins/otel-http/$plugin_file"
done

if [[ "$COPY_OPENSSL" == "1" ]]; then
  openssl_source="$(find_openssl || true)"
  if [[ -n "$openssl_source" ]]; then
    validate_elf "$openssl_source" "$TARGET_MACHINE" "openssl"
    install -m 0755 "$openssl_source" "$BUNDLE_DIR/openssl"
    ELF_INPUTS+=("$BUNDLE_DIR/openssl")
  else
    echo "warning: openssl not found in BUNDLE_SYSROOT; guest image must provide it" >&2
  fi
fi

declare -a dependency_queue=()
declare -A dependency_seen=()

enqueue_dependencies() {
  local path="$1"
  local soname=""
  while IFS= read -r soname; do
    [[ -n "$soname" ]] || continue
    is_base_runtime_library "$soname" && continue
    [[ "$soname" != */* ]] || fail "unsafe ELF dependency name '$soname' in $path"
    if [[ -z "${dependency_seen[$soname]+x}" ]]; then
      dependency_seen["$soname"]=queued
      dependency_queue+=("$soname")
    fi
  done < <(elf_needed "$path")
}

for artifact in "${ELF_INPUTS[@]}"; do
  enqueue_dependencies "$artifact"
done

dependency_index=0
while [[ "$dependency_index" -lt "${#dependency_queue[@]}" ]]; do
  soname="${dependency_queue[$dependency_index]}"
  dependency_index=$((dependency_index + 1))
  library_source="$(find_library "$soname" "$TARGET_MACHINE" || true)"
  [[ -n "$library_source" ]] \
    || fail "cannot resolve dependency $soname for '$TARGET_MACHINE' under BUNDLE_SYSROOT=$BUNDLE_SYSROOT"
  validate_elf "$library_source" "$TARGET_MACHINE" "dependency $soname"
  install -m 0644 "$library_source" "$BUNDLE_DIR/lib/$soname"
  dependency_seen["$soname"]=copied
  enqueue_dependencies "$library_source"
done

cat >"$BUNDLE_DIR/openssl.cnf" <<'EOF'
openssl_conf = openssl_init

[openssl_init]
providers = provider_sect

[provider_sect]
default = default_sect

[default_sect]
activate = 1
EOF

write_config() {
  local name="$1"
  local ebpf_enabled="$2"
  local tls_enabled="$3"
  local capabilities="$4"
  local config="$BUNDLE_DIR/guest-$name.conf"

  cat >"$config" <<EOF
[control]
socket_path = "/tmp/actrail-e2e/$name/control.sock"
socket_mode_octal = "660"
pid_file = "/tmp/actrail-e2e/$name/actraild.pid"
log_path = "/tmp/actrail-e2e/$name/actraild.log"
diagnostic_log_level = "info"

[storage]
backend = "sqlite"

[storage.sqlite]
path = "/tmp/actrail-e2e/$name/actrail.sqlite"
busy_timeout_ms = 5000

[semantic_retention]
content_owner = "configured_layers"

[semantic_retention.l4_payload]
enabled = true
stats = true
body_content = "retained"

[export.snapshot]
graph_schema_version = "manual-v1"
allow_active_trace_snapshot = true
directory = "/tmp/actrail-e2e/$name/export"
payload_bytes_enabled = false
payload_text_enabled = false

[plugins.startup]
enabled = false

[capture]
profile_name = "container-auto"
capabilities = [
$capabilities
]
opportunistic_capabilities = []

[ebpf]
enabled = $ebpf_enabled
memlock_rlimit = "inherit"
tracked_process_max_entries = 4096
pending_operation_max_entries = 4096
suppressed_fd_max_entries = 4096
suppressed_fd_index_slots_per_process = 64
event_ring_buffer_max_bytes = 1048576
file_path_capture_enabled = true
file_path_max_bytes = 255

[payload.tls]
enabled = $tls_enabled
capture_backend = "tls-sync"
source = "auto"
resolver = "auto"
library = "auto"
library_path = "auto"
binary_path = "disabled"
pattern_path = "disabled"
max_segment_bytes = 4095
max_operation_bytes = 4194304
ring_buffer_bytes = 1048576
pending_operation_max_entries = 4096
retention_max_bytes_per_trace = 10485760
redaction_policy = "authorization-header"
sync_runtime_library_path = "auto"
sync_event_socket_path = "/tmp/actrail-e2e/$name/tls-sync.sock"
sync_socket_mode_octal = "660"
sync_match_limit = 8
seccomp_syscalls = ["write", "writev", "sendto", "sendmsg"]

[payload.stdio]
enabled = false
capture_stdin = false
capture_stdout = true
capture_stderr = true
stdin_storage_mode = "full"
stdout_storage_mode = "drop"
stderr_storage_mode = "metadata-only"
max_segment_bytes = 4095
ring_buffer_bytes = 1048576
pending_operation_max_entries = 4096
stream_state_max_entries = 4096
retention_max_bytes_per_trace = 10485760
redaction_policy = "authorization-header"

[payload.socket]
enabled = false
capture_backend = "bpf-copy-seccomp-fallback"
max_segment_bytes = 4095
max_operation_bytes = 4194304
ring_buffer_bytes = 2097152
pending_operation_max_entries = 4096
stream_state_max_entries = 4096
retention_max_bytes_per_trace = 10485760
redaction_policy = "authorization-header"
http_sniff_max_bytes = 8192
seccomp_syscalls = ["write", "sendto"]

[seccomp_notify]
enabled = true
reserved_listener_fd = 253

[process_seccomp]
enabled = false
syscalls = ["execve", "execveat", "fork", "vfork", "clone", "clone3"]
max_args = 64
max_arg_bytes = 4096
pending_max_entries = 4096

[application]
enabled = true
http1_enabled = true
http2_enabled = true

[application.http]
capture_host = true
sse_enabled = false
sse_data_policy = "disabled"
sse_max_buffer_bytes = 4194304
sse_max_data_bytes = 4096

[application.http2]
max_frame_bytes = 16384
max_connection_buffer_bytes = 1048576
emit_data_preview = false
max_data_preview_bytes = 4096

[resource_metrics]
enabled = false
interval_ms = 1000
include_children = true
include_system = true
cpu_alert_percent_millis = "disabled"
memory_alert_rss_kb = "disabled"

[provider]
rules_enabled = false
rules_path = "/etc/actrail/provider-rules.conf"
unknown_provider_label = "unknown"
EOF
}

write_config "tls-only" "false" "true" '  "tls-plaintext-payload",'

write_config "ebpf-only" "true" "false" '  "proc-lifecycle",
  "net-transport",
  "fs-access-basic",'

write_config "combo" "true" "true" '  "proc-lifecycle",
  "net-transport",
  "fs-access-basic",
  "tls-plaintext-payload",
  "net-application-plaintext-http",'

cat >"$BUNDLE_DIR/RUNBOOK.txt" <<'EOF'
Run inside a Kata guest/container:
  /actrail/tests/guest/precheck.sh
  /actrail/tests/guest/tls-only.sh
  /actrail/tests/guest/ebpf-only.sh
  /actrail/tests/guest/combo.sh

Expected success markers:
  GUEST_PRECHECK_OK
  TLS_ONLY_OK
  EBPF_ONLY_OK
  COMBO_OK
EOF

cat >"$BUNDLE_DIR/BUNDLE-INFO" <<EOF
format=1
target_machine=$TARGET_MACHINE
target_glibc_max=$TARGET_GLIBC_MAX
program_interpreter=$PROGRAM_INTERPRETER
dependency_sysroot=$BUNDLE_SYSROOT
ebpf_transport_request=$EBPF_TRANSPORT
ebpf_transport_build_applied=$ACTRAIL_BUILD
build_profile=$BUILD_PROFILE
EOF

(
  cd "$BUNDLE_DIR"
  while IFS= read -r path; do
    sha256sum "$path"
  done < <(find . -type f ! -name MANIFEST.sha256 -print | LC_ALL=C sort)
) >"$BUNDLE_DIR/MANIFEST.sha256"

backup_dir="${FINAL_BUNDLE_DIR}.previous.$$"
[[ ! -e "$backup_dir" ]] || fail "temporary backup path already exists: $backup_dir"
if [[ -e "$FINAL_BUNDLE_DIR" ]]; then
  mv "$FINAL_BUNDLE_DIR" "$backup_dir"
fi
if mv "$STAGING_DIR" "$FINAL_BUNDLE_DIR"; then
  STAGING_DIR=""
  rm -rf "$backup_dir"
else
  [[ ! -e "$FINAL_BUNDLE_DIR" && -e "$backup_dir" ]] \
    && mv "$backup_dir" "$FINAL_BUNDLE_DIR"
  fail "failed to publish bundle at $FINAL_BUNDLE_DIR"
fi
trap - EXIT

echo "bundle_ready=$FINAL_BUNDLE_DIR"
find "$FINAL_BUNDLE_DIR" -maxdepth 2 -type f | sort
