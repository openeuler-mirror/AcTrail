#!/usr/bin/env bash
# Regression checks for the minimal, daemon-free workload integration bundle.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
PREPARE="$ROOT_DIR/deploy/virtual-container/workload/prepare-bundle.sh"
GUEST_BUNDLE="${BUNDLE_DIR:-$ROOT_DIR/.actrail-guest-bundle}"
WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/actrail-workload-bundle-test.XXXXXX")"
OUTPUT="$WORK_DIR/workload"

cleanup() {
  rm -rf "$WORK_DIR"
}
trap cleanup EXIT

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

[[ -f "$GUEST_BUNDLE/MANIFEST.sha256" ]] \
  || fail "guest bundle is missing; run the virtual_container V2 case first"

"$PREPARE" \
  --guest-bundle "$GUEST_BUNDLE" \
  --output "$OUTPUT" \
  --socket-gid 39000

for relative in \
  bin/actrailctl \
  bin/actrailctl-private \
  bin/actrail-init \
  bin/actrail-launch \
  bin/verify-interface \
  etc/actrail/operator.conf \
  lib/libactrail_tls_payload_probe_sync.so \
  WORKLOAD-INTERFACE \
  MANIFEST.sha256; do
  [[ -f "$OUTPUT/$relative" ]] || fail "workload artifact missing: $relative"
done

[[ ! -e "$OUTPUT/actraild" && ! -e "$OUTPUT/bin/actraild" ]] \
  || fail "workload bundle must not contain actraild"
[[ ! -e "$OUTPUT/actrailviewer" && ! -e "$OUTPUT/bin/actrailviewer" ]] \
  || fail "workload bundle must not contain actrailviewer"
if find "$OUTPUT" -type f -name '*.sqlite' -print -quit | grep . >/dev/null; then
  fail "workload bundle contains daemon storage"
fi

grep -Fqx 'socket_path = "/run/actrail/control.sock"' \
  "$OUTPUT/etc/actrail/operator.conf" \
  || fail "workload config has the wrong control socket"
grep -Fqx 'sync_event_socket_path = "/run/actrail/tls-sync.sock"' \
  "$OUTPUT/etc/actrail/operator.conf" \
  || fail "workload config has the wrong TLS socket"
grep -Fqx \
  'sync_runtime_library_path = "/opt/actrail/lib/libactrail_tls_payload_probe_sync.so"' \
  "$OUTPUT/etc/actrail/operator.conf" \
  || fail "workload config has the wrong TLS probe path"
for section in payload.socket seccomp_notify process_seccomp enforcement; do
  awk -v section="[$section]" '
    $0 == section { inside = 1; next }
    inside && /^\[/ { exit }
    inside && $0 == "enabled = false" { found = 1 }
    END { exit !found }
  ' "$OUTPUT/etc/actrail/operator.conf" \
    || fail "Kata workload config must disable $section"
done
grep -Fq -- '--seccomp-notify disabled' "$OUTPUT/bin/actrail-launch" \
  || fail "Kata workload launcher must not auto-enable seccomp user-notify"
grep -Fq -- 'ACTRAIL_WORKLOAD_EXIT_GRACE_SECONDS' "$OUTPUT/bin/actrail-launch" \
  || fail "Kata workload launcher has no bounded guest drain window"
[[ ! -e "$OUTPUT/bin/assert-pid-namespace" ]] \
  || fail "workload bundle contains the V2-only PID namespace assertion"

grep -Fqx 'format=1' "$OUTPUT/WORKLOAD-INTERFACE" \
  || fail "workload interface format is missing"
grep -Fqx 'guest_socket_source=/dev/actrail' "$OUTPUT/WORKLOAD-INTERFACE" \
  || fail "guest socket source contract is missing"
grep -Fqx 'workload_socket_target=/run/actrail' "$OUTPUT/WORKLOAD-INTERFACE" \
  || fail "workload socket target contract is missing"
grep -Fqx 'socket_gid=39000' "$OUTPUT/WORKLOAD-INTERFACE" \
  || fail "workload socket GID contract is missing"
grep -Eq '^program_interpreter=/' "$OUTPUT/WORKLOAD-INTERFACE" \
  || fail "workload ELF interpreter contract is missing"
if grep -Fq -- 'export LD_LIBRARY_PATH' "$OUTPUT/bin/actrail-launch"; then
  fail "Kata workload launcher leaks its private library path into the Agent"
fi

(cd "$OUTPUT" && sha256sum --strict --check MANIFEST.sha256 >/dev/null)
"$OUTPUT/bin/verify-interface" --root "$OUTPUT" --artifacts-only

# The workload image is intentionally minimal. Runtime verification must not
# depend on findutils: daemon/viewer exclusion is enforced from the already
# checksum-validated manifest.
MINIMAL_PATH="$WORK_DIR/minimal-path"
mkdir -p "$MINIMAL_PATH"
for command_name in awk id sed sha256sum stat; do
  ln -s "$(command -v "$command_name")" "$MINIMAL_PATH/$command_name"
done
PATH="$MINIMAL_PATH" \
  "$OUTPUT/bin/verify-interface" --root "$OUTPUT" --artifacts-only

FORBIDDEN_OUTPUT="$WORK_DIR/workload-with-daemon"
cp -a "$OUTPUT" "$FORBIDDEN_OUTPUT"
touch "$FORBIDDEN_OUTPUT/bin/actraild"
(
  cd "$FORBIDDEN_OUTPUT"
  while IFS= read -r path; do
    sha256sum "$path"
  done < <(find . -type f ! -name MANIFEST.sha256 -print | LC_ALL=C sort)
) >"$FORBIDDEN_OUTPUT/MANIFEST.sha256"
set +e
forbidden_output="$(
  "$FORBIDDEN_OUTPUT/bin/verify-interface" \
    --root "$FORBIDDEN_OUTPUT" --artifacts-only 2>&1
)"
forbidden_rc=$?
set -e
[[ "$forbidden_rc" -ne 0 ]] \
  || fail "runtime verifier accepted a manifest containing actraild"
grep -Fq 'workload bundle contains a system daemon or viewer' \
  <<<"$forbidden_output" \
  || fail "runtime verifier rejected actraild without the expected diagnostic"

[[ "$(stat -c '%a' "$OUTPUT")" == "755" ]] \
  || fail "workload bundle root must be traversable when mounted for a non-root workload"

if find "$OUTPUT" -type f -perm /022 -print -quit | grep . >/dev/null; then
  fail "workload bundle contains group/world-writable files"
fi

# The launcher must preserve actrailctl's exit status and force the tested
# seccomp profile. Use fakes so this contract does not require a live daemon.
fake_root="$WORK_DIR/fake-root"
install -d "$fake_root/bin" "$fake_root/etc/actrail" "$fake_root/lib"
install -m 0755 "$OUTPUT/bin/actrailctl-private" "$fake_root/bin/actrailctl-private"
cat >"$fake_root/bin/verify-interface" <<'EOF'
#!/bin/sh
exit 0
EOF
cat >"$fake_root/bin/fake-loader" <<'EOF'
#!/bin/sh
case "$1" in
  --verify)
    printf '%s\n' "$2" >"$ACTRAIL_TEST_VERIFY_PATH"
    exit "${ACTRAIL_TEST_VERIFY_EXIT:-0}"
    ;;
  --library-path)
    printf '%s\n' "$2" >"$ACTRAIL_TEST_LOADER_PATH"
    shift 2
    if [ "$1" = "--list" ]; then
      printf '%s\n' "$2" >"$ACTRAIL_TEST_LIST_PATH"
      exit "${ACTRAIL_TEST_LIST_EXIT:-0}"
    fi
    exec "$@"
    ;;
  *)
    exit 90
    ;;
esac
EOF
cat >"$fake_root/bin/actrailctl" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" >"$ACTRAIL_TEST_ARGS"
printf '%s\n' "${LD_LIBRARY_PATH-}" >"$ACTRAIL_TEST_LD_PATH"
exit "${ACTRAIL_TEST_EXIT:-0}"
EOF
chmod 0755 \
  "$fake_root/bin/verify-interface" \
  "$fake_root/bin/fake-loader" \
  "$fake_root/bin/actrailctl"
: >"$fake_root/etc/actrail/operator.conf"
cat >"$fake_root/WORKLOAD-INTERFACE" <<EOF
format=1
program_interpreter=$fake_root/bin/fake-loader
EOF
set +e
ACTRAIL_WORKLOAD_ROOT="$fake_root" \
ACTRAIL_WORKLOAD_EXIT_GRACE_SECONDS=0 \
ACTRAIL_TEST_ARGS="$WORK_DIR/launch-args" \
ACTRAIL_TEST_LD_PATH="$WORK_DIR/launch-ld-path" \
ACTRAIL_TEST_LOADER_PATH="$WORK_DIR/loader-path" \
ACTRAIL_TEST_VERIFY_PATH="$WORK_DIR/verify-path" \
ACTRAIL_TEST_LIST_PATH="$WORK_DIR/list-path" \
ACTRAIL_TEST_EXIT=7 \
LD_LIBRARY_PATH=/agent/original \
  "$OUTPUT/bin/actrail-launch" --name contract -- /bin/true
launcher_rc=$?
set -e
[[ "$launcher_rc" -eq 7 ]] || fail "workload launcher lost actrailctl exit status 7"
grep -Fqx -- '--seccomp-notify' "$WORK_DIR/launch-args" \
  || fail "workload launcher omitted --seccomp-notify"
grep -Fqx -- 'disabled' "$WORK_DIR/launch-args" \
  || fail "workload launcher did not force disabled seccomp-notify"
grep -Fqx -- "$fake_root/lib" "$WORK_DIR/loader-path" \
  || fail "private loader did not receive the AcTrail library directory"
grep -Fqx -- "$fake_root/bin/actrailctl" "$WORK_DIR/verify-path" \
  || fail "private loader did not verify the bundled actrailctl ABI"
grep -Fqx -- "$fake_root/bin/actrailctl" "$WORK_DIR/list-path" \
  || fail "private loader did not resolve the bundled actrailctl runtime"
grep -Fqx -- '/agent/original' "$WORK_DIR/launch-ld-path" \
  || fail "workload launcher changed LD_LIBRARY_PATH inherited by the Agent"

# An incompatible loader/ELF pair must fail before the client starts and name
# the ABI boundary explicitly.
set +e
abi_output="$(
  ACTRAIL_WORKLOAD_ROOT="$fake_root" \
  ACTRAIL_TEST_VERIFY_PATH="$WORK_DIR/failed-verify-path" \
  ACTRAIL_TEST_VERIFY_EXIT=8 \
    "$OUTPUT/bin/actrailctl-private" doctor 2>&1
)"
abi_rc=$?
set -e
[[ "$abi_rc" -ne 0 ]] || fail "private client accepted an incompatible loader/ELF pair"
grep -Fq 'workload ABI verification failed' <<<"$abi_output" \
  || fail "loader/ELF rejection did not identify the workload ABI boundary"

# Runtime-library incompatibility is distinct from the ELF/architecture check.
set +e
runtime_output="$(
  ACTRAIL_WORKLOAD_ROOT="$fake_root" \
  ACTRAIL_TEST_VERIFY_PATH="$WORK_DIR/runtime-verify-path" \
  ACTRAIL_TEST_LOADER_PATH="$WORK_DIR/runtime-loader-path" \
  ACTRAIL_TEST_LIST_PATH="$WORK_DIR/failed-list-path" \
  ACTRAIL_TEST_LIST_EXIT=9 \
    "$OUTPUT/bin/actrailctl-private" doctor 2>&1
)"
runtime_rc=$?
set -e
[[ "$runtime_rc" -ne 0 ]] || fail "private client accepted incompatible runtime libraries"
grep -Fq 'workload runtime compatibility check failed' <<<"$runtime_output" \
  || fail "runtime-library rejection did not identify the loader boundary"

cat >"$fake_root/bin/actrail-launch" <<'EOF'
#!/bin/sh
printf '%s\n' "$@" >"$ACTRAIL_TEST_INIT_ARGS"
exit "${ACTRAIL_TEST_INIT_EXIT:-0}"
EOF
chmod 0755 "$fake_root/bin/actrail-launch"
set +e
ACTRAIL_WORKLOAD_ROOT="$fake_root" \
ACTRAIL_TEST_INIT_ARGS="$WORK_DIR/init-args" \
ACTRAIL_TEST_INIT_EXIT=9 \
  "$OUTPUT/bin/actrail-init" --name init-contract -- /bin/true
init_rc=$?
set -e
[[ "$init_rc" -eq 9 ]] || fail "workload init lost actrail-launch exit status 9"
grep -Fqx -- 'init-contract' "$WORK_DIR/init-args" \
  || fail "workload init did not forward launch arguments"

printf 'stale\n' >"$OUTPUT/stale"
"$PREPARE" \
  --guest-bundle "$GUEST_BUNDLE" \
  --output "$OUTPUT" \
  --socket-gid 39000 >/dev/null
[[ ! -e "$OUTPUT/stale" ]] || fail "stale workload artifact survived rebuild"

ln -s "$OUTPUT" "$WORK_DIR/output-link"
set +e
symlink_output="$(
  "$PREPARE" \
    --guest-bundle "$GUEST_BUNDLE" \
    --output "$WORK_DIR/output-link" \
    --socket-gid 39000 2>&1
)"
symlink_rc=$?
set -e
[[ "$symlink_rc" -ne 0 ]] || fail "symbolic-link output was accepted"
grep -Fq 'output must not be a symbolic link' <<<"$symlink_output" \
  || fail "symbolic-link rejection did not explain the unsafe path"

echo "PASS: workload integration bundle"
