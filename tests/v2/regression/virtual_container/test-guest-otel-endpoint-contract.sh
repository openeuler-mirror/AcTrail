#!/usr/bin/env bash
# Offline contract test for explicit Guest OTLP/HTTP endpoint injection.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
HELPER="$ROOT_DIR/deploy/virtual-container/guest/otel-endpoint.sh"
INSTALLER="$ROOT_DIR/deploy/virtual-container/guest/install-rootfs.sh"
INJECTOR="$ROOT_DIR/deploy/virtual-container/guest/inject-image.sh"
VERIFIER="$ROOT_DIR/deploy/virtual-container/guest/verify-rootfs.sh"
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
[[ "$(grep -Fc -- '--otel-endpoint "$OTEL_ENDPOINT"' "$INJECTOR")" == "2" ]] \
  || fail "image injector must forward the endpoint to both installer and verifier"
grep -Fq -- 'actrail_write_guest_otel_endpoint_config' "$INSTALLER" \
  || fail "rootfs installer does not render the endpoint into plugin config"
grep -Fq -- 'endpoint = \"$EXPECTED_OTEL_ENDPOINT\"' "$VERIFIER" \
  || fail "rootfs verifier does not assert the injected endpoint"

echo "PASS: explicit Guest OTLP/HTTP endpoint contract"
