#!/usr/bin/env bash
# Regression test for the offline Kata guest-root installation contract.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
BUNDLE_DIR="${BUNDLE_DIR:-$ROOT_DIR/.actrail-guest-bundle}"
INSTALLER="$ROOT_DIR/deploy/virtual-container/guest/install-rootfs.sh"
VERIFIER="$ROOT_DIR/deploy/virtual-container/guest/verify-rootfs.sh"
TEST_OTEL_ENDPOINT="http://192.0.2.10:4318/v1/traces"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

[[ -f "$BUNDLE_DIR/MANIFEST.sha256" ]] \
  || fail "guest bundle is missing; run the virtual_container V2 case first"

# Parse the production guest template through the real CLI. This catches
# stale TOML fields before an image is built.
LD_LIBRARY_PATH="$BUNDLE_DIR/lib" \
  "$BUNDLE_DIR/actrailctl" \
  --config "$ROOT_DIR/deploy/virtual-container/guest/operator.conf" \
  probe --skip-daemon --json >/dev/null

systemd_binary=""
for candidate in /usr/lib/systemd/systemd /lib/systemd/systemd; do
  if [[ -f "$candidate" ]]; then
    systemd_binary="$(readlink -f "$candidate")"
    break
  fi
done
[[ -n "$systemd_binary" ]] || fail "host systemd binary not found"

libc_binary=""
while IFS= read -r candidate; do
  libc_binary="$candidate"
  break
done < <(
  find /usr/lib /usr/lib64 /lib /lib64 \
    -xdev -type f -name libc.so.6 -print 2>/dev/null \
    | LC_ALL=C sort -u
)
[[ -n "$libc_binary" ]] || fail "host libc.so.6 not found"

test_dir="$(mktemp -d "${TMPDIR:-/tmp}/actrail-rootfs-test.XXXXXX")"
cleanup() {
  rm -rf "$test_dir"
}
trap cleanup EXIT

rootfs="$test_dir/rootfs"
install -d \
  "$rootfs/usr/lib/systemd/system" \
  "$rootfs/usr/lib/actrail-test" \
  "$rootfs/etc"
install -m 0755 "$systemd_binary" "$rootfs/usr/lib/systemd/systemd"
install -m 0755 "$libc_binary" "$rootfs/usr/lib/actrail-test/libc.so.6"
printf 'root:x:0:\n' >"$rootfs/etc/group"
: >"$rootfs/usr/lib/systemd/system/kata-containers.target"
: >"$rootfs/usr/lib/systemd/system/kata-agent.service"

# The old observation-failure terminology must not remain as an install alias.
set +e
legacy_mode_output="$(
  "$INSTALLER" \
    --rootfs "$rootfs" \
    --bundle "$BUNDLE_DIR" \
    --mode fail-open 2>&1
)"
legacy_mode_rc=$?
set -e
[[ "$legacy_mode_rc" -ne 0 ]] || fail "installer retained the deprecated --mode alias"
grep -Fq 'unknown argument: --mode' <<<"$legacy_mode_output" \
  || fail "deprecated --mode rejection did not explain the interface change"

# VSOCK is an OTLP transport and therefore cannot be selected without enabling
# the exporter. Reject this combination before changing the rootfs.
set +e
missing_endpoint_output="$(
  "$INSTALLER" \
    --rootfs "$rootfs" \
    --bundle "$BUNDLE_DIR" \
    --egress-mode vsock-bridge 2>&1
)"
missing_endpoint_rc=$?
set -e
[[ "$missing_endpoint_rc" -ne 0 ]] \
  || fail "installer accepted vsock-bridge without --otel-endpoint"
grep -Fq -- '--egress-mode vsock-bridge requires --otel-endpoint' \
  <<<"$missing_endpoint_output" \
  || fail "missing endpoint rejection did not explain the VSOCK dependency"
[[ ! -e "$rootfs/usr/local/bin/actraild" ]] \
  || fail "installer wrote artifacts before rejecting the missing endpoint"

# A numeric GID collision must fail before any AcTrail artifact is installed.
collision_rootfs="$test_dir/collision-rootfs"
cp -a "$rootfs" "$collision_rootfs"
printf 'already-used:x:39000:\n' >>"$collision_rootfs/etc/group"
set +e
collision_output="$(
  "$INSTALLER" \
    --rootfs "$collision_rootfs" \
    --bundle "$BUNDLE_DIR" \
    --otel-endpoint "$TEST_OTEL_ENDPOINT" \
    --startup-dependency optional 2>&1
)"
collision_rc=$?
set -e
[[ "$collision_rc" -ne 0 ]] || fail "installer accepted an occupied socket GID"
grep -Fq 'GID 39000 is already used by group already-used' <<<"$collision_output" \
  || fail "socket GID collision did not produce the expected diagnostic"
[[ ! -e "$collision_rootfs/usr/local/bin/actraild" ]] \
  || fail "installer wrote artifacts before rejecting the socket GID collision"

# The default installation is local-only: SQLite remains available, while the
# exporter files and startup entry are absent.
"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --startup-dependency optional
"$VERIFIER" \
  --rootfs "$rootfs" \
  --startup-dependency optional
grep -Fqx 'path = "/run/actrail/private/actrail.sqlite"' \
  "$rootfs/etc/actrail/operator.conf" \
  || fail "local-only install does not retain Guest SQLite"
if grep -Fq 'kata-guest.otel-http' "$rootfs/etc/actrail/operator.conf"; then
  fail "local-only install unexpectedly loads otel-http"
fi
[[ ! -e "$rootfs/etc/actrail/plugins/otel-http/otel-http.config.toml" ]] \
  || fail "local-only install unexpectedly contains exporter config"

"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --otel-endpoint "$TEST_OTEL_ENDPOINT" \
  --startup-dependency optional
"$VERIFIER" \
  --rootfs "$rootfs" \
  --otel-endpoint "$TEST_OTEL_ENDPOINT" \
  --startup-dependency optional
grep -Fqx -- "endpoint = \"$TEST_OTEL_ENDPOINT\"" \
  "$rootfs/etc/actrail/plugins/otel-http/otel-http.config.toml" \
  || fail "installer did not inject the requested OTLP/HTTP endpoint"
if grep -Fq -- 'COLLECTOR_HOST' \
  "$rootfs/etc/actrail/plugins/otel-http/otel-http.config.toml"; then
  fail "installed OTLP/HTTP config retained the bundle placeholder"
fi
# Parse the rendered exporter startup fragment through the real CLI. Rewrite
# only offline-root paths so this checks the candidate rootfs rather than files
# that may happen to be installed on the test host.
rendered_probe_config="$test_dir/rendered-otel-operator.conf"
sed \
  -e "s#/usr/local/lib/actrail/libactrail_tls_payload_probe_sync.so#$BUNDLE_DIR/libactrail_tls_payload_probe_sync.so#" \
  -e "s#/usr/share/actrail/plugins/otel-http/otel-http.plugin.toml#$rootfs/usr/share/actrail/plugins/otel-http/otel-http.plugin.toml#" \
  -e "s#/etc/actrail/plugins/otel-http/otel-http.config.toml#$rootfs/etc/actrail/plugins/otel-http/otel-http.config.toml#" \
  "$rootfs/etc/actrail/operator.conf" >"$rendered_probe_config"
LD_LIBRARY_PATH="$BUNDLE_DIR/lib" \
  "$BUNDLE_DIR/actrailctl" \
  --config "$rendered_probe_config" \
  probe --skip-daemon --json >/dev/null
[[ ! -e "$rootfs/usr/local/bin/actrailviewer" ]] \
  || fail "minimal guest install unexpectedly includes actrailviewer"

"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --otel-endpoint "$TEST_OTEL_ENDPOINT" \
  --startup-dependency required \
  --with-viewer
"$VERIFIER" \
  --rootfs "$rootfs" \
  --otel-endpoint "$TEST_OTEL_ENDPOINT" \
  --startup-dependency required
[[ -x "$rootfs/usr/local/bin/actrailviewer" ]] \
  || fail "--with-viewer did not install actrailviewer"

# Returning to an optional dependency removes only AcTrail's kata-agent drop-in.
"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --otel-endpoint "$TEST_OTEL_ENDPOINT" \
  --startup-dependency optional
"$VERIFIER" \
  --rootfs "$rootfs" \
  --otel-endpoint "$TEST_OTEL_ENDPOINT" \
  --startup-dependency optional

# Returning to the default local-only mode removes exporter configuration and
# leaves the base observation service usable.
"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --startup-dependency optional
"$VERIFIER" \
  --rootfs "$rootfs" \
  --startup-dependency optional
[[ ! -e "$rootfs/etc/actrail/plugins/otel-http/otel-http.config.toml" ]] \
  || fail "local-only reinstall retained exporter config"

# Execution-isolation images install the sandbox observer into the Guest
# system. The public installer/verifier pair owns the complete contract so
# every Kata VMM receives the same service topology.
"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --startup-dependency optional \
  --with-sandbox-observer
"$VERIFIER" \
  --rootfs "$rootfs" \
  --startup-dependency optional \
  --with-sandbox-observer
grep -Fq -- '/dev/actrail/sandbox-observer.ready' \
  "$rootfs/usr/lib/systemd/system/actrail-sb.service" \
  || fail "sandbox observer does not publish workload-visible readiness"
grep -Fqx -- \
  'socket_path = "/dev/actrail/sandbox-observer-control.sock"' \
  "$rootfs/etc/actrail/sandbox-observer.toml" \
  || fail "sandbox observer does not publish its control socket through /dev/actrail"
grep -Fq -- '/dev/actrail/sandbox-observer-control.sock' \
  "$rootfs/usr/lib/systemd/system/actrail-sb-connect.service" \
  || fail "sandbox observer connect unit does not use the published control socket"
grep -Fqx -- 'WantedBy=multi-user.target kata-containers.target' \
  "$rootfs/usr/lib/systemd/system/actrail-sb-connect.service" \
  || fail "sandbox observer connect unit cannot be enabled for production"
for target in kata-containers.target multi-user.target; do
  connect_link="$rootfs/usr/lib/systemd/system/$target.wants/actrail-sb-connect.service"
  [[ ! -e "$connect_link" && ! -L "$connect_link" ]] \
    || fail "sandbox observer auto-connect is enabled before case orchestration: $connect_link"
done
[[ ! -e "$rootfs/usr/lib/systemd/system/kata-agent.service.d/30-actrail-sandbox-observer.conf" ]] \
  || fail "sandbox observer still changes kata-agent startup ordering"

# Reinstalling the normal image profile must remove the observer topology
# symmetrically rather than leaving a stale root service behind.
"$INSTALLER" \
  --rootfs "$rootfs" \
  --bundle "$BUNDLE_DIR" \
  --startup-dependency optional
for observer_path in \
  /usr/local/bin/actrail-sb \
  /etc/actrail/sandbox-observer.toml \
  /usr/lib/systemd/system/actrail-sb.service \
  /usr/lib/systemd/system/actrail-sb-connect.service \
  /usr/lib/systemd/system/kata-agent.service.d/30-actrail-sandbox-observer.conf \
  /usr/lib/systemd/system/kata-containers.target.wants/actrail-sb.service \
  /usr/lib/systemd/system/kata-containers.target.wants/actrail-sb-connect.service \
  /usr/lib/systemd/system/multi-user.target.wants/actrail-sb.service \
  /usr/lib/systemd/system/multi-user.target.wants/actrail-sb-connect.service; do
  [[ ! -e "$rootfs$observer_path" && ! -L "$rootfs$observer_path" ]] \
    || fail "default reinstall retained sandbox observer path: $observer_path"
done
"$VERIFIER" \
  --rootfs "$rootfs" \
  --startup-dependency optional

echo "PASS: guest rootfs installer"
