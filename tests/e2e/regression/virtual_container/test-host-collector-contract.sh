#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
ASSET_DIR="$ROOT_DIR/deploy/virtual-container/host-collector"
COMPOSE_FILE="$ASSET_DIR/compose.yaml"
CONFIG_FILE="$ASSET_DIR/otelcol-contrib.yaml"
ENV_EXAMPLE="$ASSET_DIR/.env.example"
README_FILE="$ASSET_DIR/README.md"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

assert_file() {
  [[ -f "$1" ]] || fail "missing file: $1"
}

assert_line() {
  local file="$1"
  local pattern="$2"
  local description="$3"

  grep -Eq "$pattern" "$file" || fail "$description"
}

for required_file in "$COMPOSE_FILE" "$CONFIG_FILE" "$ENV_EXAMPLE" "$README_FILE"; do
  assert_file "$required_file"
done

assert_line "$COMPOSE_FILE" '^[[:space:]]*image:[[:space:]]*otel/opentelemetry-collector-contrib:0\.157\.0[[:space:]]*$' \
  "Collector image must be pinned to the official 0.157.0 tag"
if grep -Eq '^[[:space:]]*image:.*(:latest|otelcol-contrib:[[:space:]]*$)' "$COMPOSE_FILE"; then
  fail "Collector image must not use an unpinned tag"
fi
assert_line "$COMPOSE_FILE" '^[[:space:]]*network_mode:[[:space:]]*host[[:space:]]*$' \
  "Collector must use host networking"
if grep -Eq '^[[:space:]]*ports:[[:space:]]*$' "$COMPOSE_FILE"; then
  fail "host-network deployment must not publish Compose ports"
fi
assert_line "$COMPOSE_FILE" '^[[:space:]]*user:[[:space:]]*"10001:10001"[[:space:]]*$' \
  "Collector must run as uid:gid 10001:10001"
assert_line "$COMPOSE_FILE" '^[[:space:]]*read_only:[[:space:]]*true[[:space:]]*$' \
  "Collector root filesystem must be read-only"
assert_line "$COMPOSE_FILE" '^[[:space:]]*cap_drop:[[:space:]]*$' \
  "Collector must drop capabilities"
assert_line "$COMPOSE_FILE" '^[[:space:]]*-[[:space:]]*ALL[[:space:]]*$' \
  "Collector must drop all capabilities"
assert_line "$COMPOSE_FILE" '^[[:space:]]*-[[:space:]]*no-new-privileges:true[[:space:]]*$' \
  "Collector must enable no-new-privileges"
assert_line "$COMPOSE_FILE" '^[[:space:]]*mem_limit:[[:space:]]*384m[[:space:]]*$' \
  "Collector must have a cgroup memory limit"
assert_line "$COMPOSE_FILE" '^[[:space:]]*memswap_limit:[[:space:]]*384m[[:space:]]*$' \
  "Collector must not add swap headroom beyond the memory limit"
assert_line "$COMPOSE_FILE" '^[[:space:]]*GOMEMLIMIT:[[:space:]]*320MiB[[:space:]]*$' \
  "Collector must align the Go runtime limit below its cgroup memory limit"
assert_line "$COMPOSE_FILE" '^[[:space:]]*driver:[[:space:]]*json-file[[:space:]]*$' \
  "Collector container logs must use an explicitly bounded logging driver"
assert_line "$COMPOSE_FILE" '^[[:space:]]*max-size:[[:space:]]*"16m"[[:space:]]*$' \
  "Collector container logs must set max-size"
assert_line "$COMPOSE_FILE" '^[[:space:]]*max-file:[[:space:]]*"3"[[:space:]]*$' \
  "Collector container logs must set max-file"
assert_line "$COMPOSE_FILE" 'OTELCOL_OTLP_HTTP_ENDPOINT:.*\$\{OTELCOL_OTLP_HTTP_ENDPOINT:\?' \
  "OTLP/HTTP listen address must be a required environment setting"
assert_line "$COMPOSE_FILE" 'source:.*\$\{OTELCOL_DATA_DIR:\?' \
  "writable data directory must be a required environment setting"
assert_line "$COMPOSE_FILE" '^[[:space:]]*target:[[:space:]]*/var/lib/otelcol[[:space:]]*$' \
  "data directory must be mounted at /var/lib/otelcol"
assert_line "$COMPOSE_FILE" '^[[:space:]]*read_only:[[:space:]]*false[[:space:]]*$' \
  "data directory mount must be explicitly writable"
[[ "$(grep -Ec '^[[:space:]]*create_host_path:[[:space:]]*false[[:space:]]*$' "$COMPOSE_FILE")" == "2" ]] \
  || fail "both Collector bind sources must already exist"
assert_line "$COMPOSE_FILE" '^[[:space:]]*selinux:[[:space:]]*z[[:space:]]*$' \
  "shared Collector config bind must request an SELinux relabel"
assert_line "$COMPOSE_FILE" '^[[:space:]]*selinux:[[:space:]]*Z[[:space:]]*$' \
  "private Collector data bind must request an SELinux relabel"

assert_line "$CONFIG_FILE" '^[[:space:]]*endpoint:[[:space:]]*\$\{env:OTELCOL_OTLP_HTTP_ENDPOINT\}[[:space:]]*$' \
  "OTLP/HTTP receiver must use the required environment setting"
assert_line "$CONFIG_FILE" '^[[:space:]]*endpoint:[[:space:]]*127\.0\.0\.1:13133[[:space:]]*$' \
  "health endpoint must be host-loopback only"
assert_line "$CONFIG_FILE" '^[[:space:]]*memory_limiter:[[:space:]]*$' \
  "trace pipeline must define a memory limiter"
assert_line "$CONFIG_FILE" '^[[:space:]]*batch:[[:space:]]*$' \
  "trace pipeline must define batching"
assert_line "$CONFIG_FILE" '^[[:space:]]*-[[:space:]]*memory_limiter[[:space:]]*$' \
  "trace pipeline must use the memory limiter"
assert_line "$CONFIG_FILE" '^[[:space:]]*-[[:space:]]*batch[[:space:]]*$' \
  "trace pipeline must use batching"
assert_line "$CONFIG_FILE" '^[[:space:]]*debug:[[:space:]]*$' \
  "acceptance pipeline must define the debug exporter"
assert_line "$CONFIG_FILE" '^[[:space:]]*file/acceptance:[[:space:]]*$' \
  "acceptance pipeline must define a file exporter"
assert_line "$CONFIG_FILE" '^[[:space:]]*path:[[:space:]]*/var/lib/otelcol/actrail-traces\.json[[:space:]]*$' \
  "file exporter must write to the explicit data directory"
assert_line "$CONFIG_FILE" '^[[:space:]]*format:[[:space:]]*json[[:space:]]*$' \
  "file exporter must use JSON"
assert_line "$CONFIG_FILE" '^[[:space:]]*rotation:[[:space:]]*$' \
  "file exporter must enable rotation"
for rotation_key in max_megabytes max_days max_backups; do
  assert_line "$CONFIG_FILE" "^[[:space:]]*${rotation_key}:[[:space:]]*[1-9][0-9]*[[:space:]]*$" \
    "file exporter rotation must set ${rotation_key}"
done
assert_line "$CONFIG_FILE" '^[[:space:]]*-[[:space:]]*debug[[:space:]]*$' \
  "trace pipeline must use the debug exporter"
assert_line "$CONFIG_FILE" '^[[:space:]]*-[[:space:]]*file/acceptance[[:space:]]*$' \
  "trace pipeline must use the file exporter"
assert_line "$CONFIG_FILE" '^[[:space:]]*-[[:space:]]*health_check[[:space:]]*$' \
  "service must enable the health extension"

assert_line "$ENV_EXAMPLE" '^OTELCOL_OTLP_HTTP_ENDPOINT=0\.0\.0\.0:4318$' \
  ".env.example must document the development receiver address"
assert_line "$ENV_EXAMPLE" '^OTELCOL_DATA_DIR=/var/lib/actrail/otelcol$' \
  ".env.example must document the writable host data directory"

assert_line "$README_FILE" '127\.0\.0\.1.*Guest.*不是宿主机' \
  "README must warn that Guest loopback is not the host"
assert_line "$README_FILE" '明文 HTTP' \
  "README must state the development plaintext limitation"
assert_line "$README_FILE" 'curl .*127\.0\.0\.1:13133' \
  "README must document the host-local health check"
assert_line "$README_FILE" 'docker compose .* ps' \
  "README must document startup verification"
assert_line "$README_FILE" 'docker compose .*logs' \
  "README must document log verification"
assert_line "$README_FILE" 'actrail-traces\.json' \
  "README must document acceptance data verification"
assert_line "$README_FILE" '不是 WAL' \
  "README must state that the file exporter is not a WAL"
assert_line "$README_FILE" 'GOMEMLIMIT=320MiB' \
  "README must document the Go runtime memory limit"
assert_line "$README_FILE" 'cgroup.*硬边界' \
  "README must explain the cgroup hard memory boundary"
assert_line "$README_FILE" 'Docker.*日志.*轮转' \
  "README must document container-log rotation"
assert_line "$README_FILE" 'SELinux.*relabel' \
  "README must document bind-mount SELinux relabeling"

echo "PASS: host Collector deployment contract"
