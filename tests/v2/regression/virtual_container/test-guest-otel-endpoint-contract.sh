#!/usr/bin/env bash
# Offline contract test for explicit Guest OTLP/HTTP endpoint injection.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
HELPER="$ROOT_DIR/deploy/virtual-container/guest/otel-endpoint.sh"
INSTALLER="$ROOT_DIR/deploy/virtual-container/guest/install-rootfs.sh"
INJECTOR="$ROOT_DIR/deploy/virtual-container/guest/inject-image.sh"
VERIFIER="$ROOT_DIR/deploy/virtual-container/guest/verify-rootfs.sh"
BUILDER="$ROOT_DIR/deploy/virtual-container/guest/build-openeuler-image.sh"
CONFIG_TEMPLATE="$ROOT_DIR/examples/plugins/builtin/otel-http/otel-http.config.toml"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

[[ -f "$HELPER" ]] || fail "Guest endpoint helper not found: $HELPER"
# shellcheck source=../../../../deploy/virtual-container/guest/otel-endpoint.sh
source "$HELPER"

expect_valid() {
  local endpoint="$1"
  actrail_validate_guest_otel_endpoint "$endpoint" \
    || fail "valid endpoint rejected: $endpoint: $ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
}

expect_invalid() {
  local endpoint="$1"
  local expected_error="$2"

  if actrail_validate_guest_otel_endpoint "$endpoint"; then
    fail "invalid endpoint accepted: $endpoint"
  fi
  [[ "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR" == *"$expected_error"* ]] \
    || fail "unexpected endpoint diagnostic: $ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
}

HTTP_ENDPOINT="http://192.0.2.10:4318/v1/traces"
HTTPS_ENDPOINT="https://collector.internal:4318/v1/traces"
expect_valid "$HTTP_ENDPOINT"
expect_valid "$HTTPS_ENDPOINT"
expect_valid "https://collector.internal:4318/otel/gateway/v1/traces"

expect_invalid "" "required"
expect_invalid "grpc://collector.internal:4317" "http:// or https://"
expect_invalid "http://collector.internal:4318" "ending in /v1/traces"
expect_invalid "http://collector.internal:4318/v1/metrics" "end in /v1/traces"
expect_invalid "http://collector.internal:4318/v1/traces/" "end in /v1/traces"
expect_invalid "http://collector.internal:4318/v1/traces?tenant=test" \
  "query or fragment"
expect_invalid "http://collector.internal:4318/v1/traces#fragment" \
  "query or fragment"
expect_invalid "http://COLLECTOR_HOST:4318/v1/traces" "placeholder"
expect_invalid "http://127.0.0.1:4318/v1/traces" "Guest loopback"
expect_invalid "http://127.1.2.3:4318/v1/traces" "Guest loopback"
expect_invalid "http://2130706433:4318/v1/traces" "invalid or ambiguous IPv4"
expect_invalid "http://0177.0.0.1:4318/v1/traces" "invalid or ambiguous IPv4"
expect_invalid "http://999.999.999.999:4318/v1/traces" "invalid or ambiguous IPv4"
expect_invalid "http://a..b:4318/v1/traces" "invalid host"
expect_invalid "http://localhost:4318/v1/traces" "Guest loopback"
expect_invalid "http://0.0.0.0:4318/v1/traces" "Guest loopback"
expect_invalid "https://[2001:db8::10]:4318/v1/traces" \
  "IPv6 literals are not supported"
expect_invalid "https://[::::]:4318/v1/traces" \
  "IPv6 literals are not supported"
expect_invalid "http://collector.internal:70000/v1/traces" "between 1 and 65535"
expect_invalid $'http://collector.internal:4318/v1/traces"\nqueue_capacity = 999999' \
  "unsafe in the Guest TOML config"

# --- egress mode dispatch ---------------------------------------------------
# The Guest reaches the Collector either over its own network (CNI/K8s) or over
# the VSOCK bridge, which terminates on Guest loopback. Mode selects which
# destination is legitimate; every other rule stays in force for both modes.

expect_valid_mode() {
  local endpoint="$1"
  local mode="$2"
  actrail_validate_guest_otel_endpoint "$endpoint" "$mode" \
    || fail "valid $mode endpoint rejected: $endpoint: $ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
}

expect_invalid_mode() {
  local endpoint="$1"
  local mode="$2"
  local expected_error="$3"

  if actrail_validate_guest_otel_endpoint "$endpoint" "$mode"; then
    fail "invalid $mode endpoint accepted: $endpoint"
  fi
  [[ "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR" == *"$expected_error"* ]] \
    || fail "unexpected $mode diagnostic: $ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
}

# An omitted mode must keep behaving exactly like the network mode.
expect_valid_mode "$HTTP_ENDPOINT" network
expect_valid_mode "$HTTPS_ENDPOINT" network
expect_invalid_mode "http://127.0.0.1:4318/v1/traces" network "Guest loopback"
expect_invalid_mode "http://localhost:4318/v1/traces" network "Guest loopback"

# The VSOCK bridge listens on Guest loopback, so loopback is the only valid
# destination there — and the port must be explicit because it is the single
# source of truth for the bridge listen port.
VSOCK_ENDPOINT="http://127.0.0.1:14318/v1/traces"
expect_valid_mode "$VSOCK_ENDPOINT" vsock-bridge
expect_valid_mode "https://127.0.0.1:14318/v1/traces" vsock-bridge

expect_invalid_mode "http://192.0.2.10:4318/v1/traces" vsock-bridge \
  "vsock-bridge egress requires the Guest bridge loopback address"
expect_invalid_mode "http://collector.internal:4318/v1/traces" vsock-bridge \
  "vsock-bridge egress requires the Guest bridge loopback address"
expect_invalid_mode "http://0.0.0.0:14318/v1/traces" vsock-bridge \
  "vsock-bridge egress requires the Guest bridge loopback address"
# localhost depends on Guest DNS/hosts resolution; require the literal address.
expect_invalid_mode "http://localhost:14318/v1/traces" vsock-bridge \
  "vsock-bridge egress requires the Guest bridge loopback address"
expect_invalid_mode "http://127.0.0.1/v1/traces" vsock-bridge \
  "explicit port"

# Mode never weakens the shared rules.
expect_invalid_mode "http://127.0.0.1:14318/v1/metrics" vsock-bridge \
  "end in /v1/traces"
expect_invalid_mode "http://COLLECTOR_HOST:4318/v1/traces" vsock-bridge \
  "placeholder"
expect_invalid_mode "http://127.0.0.1:70000/v1/traces" vsock-bridge \
  "between 1 and 65535"
expect_invalid_mode $'http://127.0.0.1:14318/v1/traces"\nqueue_capacity = 999999' \
  vsock-bridge "unsafe in the Guest TOML config"

expect_invalid_mode "$HTTP_ENDPOINT" bogus-mode "egress mode"
expect_invalid_mode "$HTTP_ENDPOINT" "" "egress mode"

# Deployment selection is optional in network mode. VSOCK remains meaningful
# only when an exporter endpoint is present.
actrail_validate_guest_otel_selection "" network \
  || fail "local-only Guest selection was rejected"
if actrail_validate_guest_otel_selection "" vsock-bridge; then
  fail "vsock-bridge was accepted without an exporter endpoint"
fi
[[ "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR" == *"requires --otel-endpoint"* ]] \
  || fail "VSOCK without endpoint returned the wrong diagnostic"

# The bridge listen port is derived from the endpoint, never configured twice.
bridge_port="$(actrail_guest_otel_endpoint_port "$VSOCK_ENDPOINT")" \
  || fail "endpoint port accessor failed: $ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
[[ "$bridge_port" == "14318" ]] \
  || fail "endpoint port accessor returned $bridge_port, expected 14318"

test_dir="$(mktemp -d "${TMPDIR:-/tmp}/actrail-guest-otel-contract.XXXXXX")"
cleanup() {
  rm -rf "$test_dir"
}
trap cleanup EXIT

rendered_config="$test_dir/otel-http.config.toml"
actrail_write_guest_otel_endpoint_config \
  "$CONFIG_TEMPLATE" "$rendered_config" "$HTTP_ENDPOINT" \
  || fail "$ACTRAIL_GUEST_OTEL_ENDPOINT_ERROR"
grep -Fqx -- "endpoint = \"$HTTP_ENDPOINT\"" "$rendered_config" \
  || fail "rendered config does not contain the requested endpoint"
[[ "$(grep -Ec '^[[:space:]]*endpoint[[:space:]]*=' "$rendered_config")" == "1" ]] \
  || fail "rendered config must contain exactly one endpoint assignment"
if grep -Eiq -- 'COLLECTOR_HOST|placeholder|replace[_-]me|change[_-]me' \
  "$rendered_config"; then
  fail "rendered config retained the bundle placeholder"
fi
[[ "$(stat -c '%a' "$rendered_config")" == "640" ]] \
  || fail "rendered config must use mode 0640"

duplicate_template="$test_dir/duplicate.config.toml"
duplicate_target="$test_dir/duplicate.rendered.toml"
printf '%s\n' \
  'endpoint = "http://COLLECTOR_HOST:4318/v1/traces"' \
  'endpoint = "http://SECOND_HOST:4318/v1/traces"' \
  >"$duplicate_template"
if actrail_write_guest_otel_endpoint_config \
  "$duplicate_template" "$duplicate_target" "$HTTP_ENDPOINT"; then
  fail "renderer accepted multiple endpoint assignments"
fi
[[ ! -e "$duplicate_target" ]] \
  || fail "renderer published a config after rejecting the template"
if find "$test_dir" -maxdepth 1 -type f -name '*.tmp.*' | grep -q .; then
  fail "renderer left a temporary config behind"
fi

invalid_image="$test_dir/must-not-be-created.img"
set +e
invalid_inject_output="$(
  "$INJECTOR" \
    --source-image "$test_dir/missing-source.img" \
    --output-image "$invalid_image" \
    --bundle "$test_dir" \
    --otel-endpoint "http://COLLECTOR_HOST:4318/v1/traces" 2>&1
)"
invalid_inject_rc=$?
set -e
[[ "$invalid_inject_rc" -ne 0 ]] \
  || fail "image injector accepted a placeholder endpoint"
grep -Fq -- 'not a placeholder' <<<"$invalid_inject_output" \
  || fail "image injector did not explain placeholder rejection"
[[ ! -e "$invalid_image" ]] \
  || fail "image injector copied output before validating the endpoint"

for script in "$INSTALLER" "$INJECTOR" "$VERIFIER"; do
  "$script" --help | grep -Fq -- '--otel-endpoint' \
    || fail "$script does not document --otel-endpoint"
done

# The egress mode travels with the endpoint through every deployment entry
# point, and is recorded in the image so verification can assert it offline.
for script in "$INSTALLER" "$INJECTOR" "$VERIFIER" "$BUILDER"; do
  "$script" --help | grep -Fq -- '--egress-mode' \
    || fail "$script does not document --egress-mode"
done
[[ "$(grep -Fc -- '--egress-mode "$EGRESS_MODE"' "$INJECTOR")" == "2" ]] \
  || fail "image injector must forward the egress mode to installer and verifier"
[[ "$(grep -Fc -- '--egress-mode "$EGRESS_MODE"' "$BUILDER")" -ge 2 ]] \
  || fail "image builder must forward the egress mode to installer and verifier"
grep -Fq 'guest_egress_mode=' "$INSTALLER" \
  || fail "rootfs installer does not record the egress mode in guest-install-info"
grep -Fq 'guest_egress_mode=$EXPECTED_EGRESS_MODE' "$VERIFIER" \
  || fail "rootfs verifier does not assert the recorded egress mode"

# vsock-bridge mode installs and enables the Guest bridge; network mode must
# leave no bridge behind. The listen port is derived from the endpoint so the
# bridge and the exporter can never disagree.
grep -Fq 'vsock-egress/guest-bridge.sh' "$INSTALLER" \
  || fail "rootfs installer does not install the Guest VSOCK bridge"
grep -Fq 'actrail_guest_otel_endpoint_port' "$INSTALLER" \
  || fail "installer does not derive the bridge listen port from the endpoint"
grep -Fq 'actrail-vsock-guest-bridge.service' "$VERIFIER" \
  || fail "rootfs verifier does not assert the Guest bridge per egress mode"
grep -Fq 'usr/bin/socat' "$VERIFIER" \
  || fail "rootfs verifier does not assert socat exists for vsock-bridge egress"
grep -Fq -- 'install_args+=(--otel-endpoint "$OTEL_ENDPOINT")' "$INJECTOR" \
  || fail "image injector does not conditionally enable the exporter"
grep -Fq -- 'verify_args+=(--otel-endpoint "$OTEL_ENDPOINT")' "$INJECTOR" \
  || fail "image injector does not conditionally verify the exporter"
grep -Fq -- 'actrail_write_guest_otel_endpoint_config' "$INSTALLER" \
  || fail "rootfs installer does not render the endpoint into plugin config"
grep -Fq -- 'endpoint = \"$EXPECTED_OTEL_ENDPOINT\"' "$VERIFIER" \
  || fail "rootfs verifier does not assert the injected endpoint"
grep -Fq -- 'otel_export_enabled=' "$INSTALLER" \
  || fail "rootfs installer does not record whether export is enabled"

echo "PASS: explicit Guest OTLP/HTTP endpoint contract"
