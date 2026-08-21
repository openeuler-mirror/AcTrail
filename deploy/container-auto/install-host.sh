#!/usr/bin/env bash
# Install the host side of the four-way Docker permission auto-selection.
#
# Usage:
#   sudo deploy/container-auto/install-host.sh [options] [BIN_DIR]
set -euo pipefail

MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${MODULE_DIR}/../.." && pwd)"
BIN_DIR="${REPO_ROOT}/target/release"
BIN_DIR_SET=0
OTEL_ENDPOINT=''
OTEL_ATTRIBUTE_MODE=metadata-only
JSONL_PLUGIN_SOURCE_DIR="${REPO_ROOT}/examples/plugins/builtin/otel-jsonl"
JSONL_PLUGIN_INSTALL_DIR="/etc/actrail/plugins/otel-jsonl"
HTTP_PLUGIN_SOURCE_DIR="${REPO_ROOT}/examples/plugins/builtin/otel-http"
HTTP_PLUGIN_INSTALL_DIR="/etc/actrail/plugins/otel-http"
SECCOMP_INSTALL_DIR="/etc/actrail/seccomp"
PROBE_LIB="libactrail_tls_payload_probe_sync.so"
RENDERED_OPERATOR_CONFIG=''

usage() {
    cat <<'EOF'
Usage: sudo deploy/container-auto/install-host.sh [options] [BIN_DIR]

Options:
  --otel-endpoint URL          Also export OTLP/HTTP spans to this Collector
  --otel-attribute-mode MODE   metadata-only or full (default: metadata-only)
  -h, --help                   Show this help
EOF
}

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

cleanup() {
    if [[ -n "$RENDERED_OPERATOR_CONFIG" && -f "$RENDERED_OPERATOR_CONFIG" ]]; then
        rm -f -- "$RENDERED_OPERATOR_CONFIG"
    fi
}
trap cleanup EXIT

while [[ "$#" -gt 0 ]]; do
    case "$1" in
        --otel-endpoint)
            [[ "$#" -ge 2 ]] || fail "--otel-endpoint requires a value"
            OTEL_ENDPOINT=$2
            shift 2
            ;;
        --otel-attribute-mode)
            [[ "$#" -ge 2 ]] || fail "--otel-attribute-mode requires a value"
            OTEL_ATTRIBUTE_MODE=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        --*)
            fail "unknown argument: $1"
            ;;
        *)
            [[ "$BIN_DIR_SET" == 0 ]] || fail "BIN_DIR may be provided only once"
            BIN_DIR=$1
            BIN_DIR_SET=1
            shift
            ;;
    esac
done

case "$OTEL_ATTRIBUTE_MODE" in
    metadata-only|full) ;;
    *) fail "--otel-attribute-mode must be metadata-only or full" ;;
esac
if [[ -z "$OTEL_ENDPOINT" && "$OTEL_ATTRIBUTE_MODE" != metadata-only ]]; then
    fail "--otel-attribute-mode requires --otel-endpoint"
fi
if [[ -n "$OTEL_ENDPOINT" ]]; then
    [[ -x "${MODULE_DIR}/render-otel-http-config.sh" ]] \
        || fail "missing executable ${MODULE_DIR}/render-otel-http-config.sh"
    "${MODULE_DIR}/render-otel-http-config.sh" \
        --template "${HTTP_PLUGIN_SOURCE_DIR}/otel-http.config.toml" \
        --endpoint "$OTEL_ENDPOINT" \
        --attribute-mode "$OTEL_ATTRIBUTE_MODE" \
        --check
fi

if [[ ${EUID} -ne 0 ]]; then
    fail "run this installer as root (sudo)"
fi

for f in actraild actrailctl actrailviewer "${PROBE_LIB}"; do
    if [[ ! -f "${BIN_DIR}/${f}" ]]; then
        echo "missing ${BIN_DIR}/${f} — build first" >&2
        exit 1
    fi
done

[[ -f "${MODULE_DIR}/seccomp/actrail-notify.json" ]] || {
    echo "missing ${MODULE_DIR}/seccomp/actrail-notify.json" >&2
    exit 1
}

for f in \
    otel-jsonl.plugin.toml \
    otel-jsonl.config.toml \
    otel-jsonl.config.v1.schema.json; do
    [[ -f "${JSONL_PLUGIN_SOURCE_DIR}/${f}" ]] \
        || fail "missing ${JSONL_PLUGIN_SOURCE_DIR}/${f}"
done
if [[ -n "$OTEL_ENDPOINT" ]]; then
    for f in \
        otel-http.plugin.toml \
        otel-http.config.toml \
        otel-http.config.v1.schema.json; do
        [[ -f "${HTTP_PLUGIN_SOURCE_DIR}/${f}" ]] \
            || fail "missing ${HTTP_PLUGIN_SOURCE_DIR}/${f}"
    done
fi

install -d -m 0755 -o root -g root /run/actrail
install -d -m 0750 -o root -g root /var/lib/actrail
install -d -m 0750 -o root -g root /var/lib/actrail/export
install -d -m 0750 -o root -g root /var/log/actrail
install -d -m 0755 -o root -g root /etc/actrail
install -d -m 0755 -o root -g root "${JSONL_PLUGIN_INSTALL_DIR}"
install -d -m 0755 -o root -g root "${SECCOMP_INSTALL_DIR}"
if [[ -n "$OTEL_ENDPOINT" ]]; then
    install -d -m 0755 -o root -g root "${HTTP_PLUGIN_INSTALL_DIR}"
fi

install -m 0755 "${BIN_DIR}/actraild" /usr/local/bin/actraild
install -m 0755 "${BIN_DIR}/actrailctl" /usr/local/bin/actrailctl
install -m 0755 "${BIN_DIR}/actrailviewer" /usr/local/bin/actrailviewer
install -m 0755 "${BIN_DIR}/${PROBE_LIB}" "/usr/local/bin/${PROBE_LIB}"

RENDERED_OPERATOR_CONFIG=$(mktemp /tmp/actrail-container-auto-conf.XXXXXX)
if [[ -n "$OTEL_ENDPOINT" ]]; then
    enable_otel_http=1
else
    enable_otel_http=0
fi
awk -v enable_otel_http="$enable_otel_http" '
    $0 == "# ACTRAIL_OTEL_HTTP_STARTUP_LOAD" {
        marker_count++
        if (enable_otel_http == "1") {
            print "[[plugins.startup.load]]"
            print "instance = \"container-auto.otel-http\""
            print "enabled = true"
            print "manifest = \"/etc/actrail/plugins/otel-http/otel-http.plugin.toml\""
            print "plugin_config = \"/etc/actrail/plugins/otel-http/otel-http.config.toml\""
            print "host_grants = []"
        }
        next
    }
    { print }
    END { if (marker_count != 1) exit 42 }
' "${MODULE_DIR}/container-auto.conf" >"${RENDERED_OPERATOR_CONFIG}" \
    || fail "container-auto.conf must contain exactly one OTEL startup marker"
install -m 0644 "$RENDERED_OPERATOR_CONFIG" /etc/actrail/container-auto.conf
install -m 0644 \
    "${JSONL_PLUGIN_SOURCE_DIR}/otel-jsonl.plugin.toml" \
    "${JSONL_PLUGIN_SOURCE_DIR}/otel-jsonl.config.toml" \
    "${JSONL_PLUGIN_SOURCE_DIR}/otel-jsonl.config.v1.schema.json" \
    "${JSONL_PLUGIN_INSTALL_DIR}/"
if [[ -n "$OTEL_ENDPOINT" ]]; then
    install -m 0644 \
        "${HTTP_PLUGIN_SOURCE_DIR}/otel-http.plugin.toml" \
        "${HTTP_PLUGIN_SOURCE_DIR}/otel-http.config.v1.schema.json" \
        "${HTTP_PLUGIN_INSTALL_DIR}/"
    "${MODULE_DIR}/render-otel-http-config.sh" \
        --template "${HTTP_PLUGIN_SOURCE_DIR}/otel-http.config.toml" \
        --output "${HTTP_PLUGIN_INSTALL_DIR}/otel-http.config.toml" \
        --endpoint "$OTEL_ENDPOINT" \
        --attribute-mode "$OTEL_ATTRIBUTE_MODE"
fi
install -m 0644 "${MODULE_DIR}/actraild.service" \
    /etc/systemd/system/actraild.service
install -m 0644 "${MODULE_DIR}/seccomp/actrail-notify.json" \
    "${SECCOMP_INSTALL_DIR}/actrail-notify.json"

systemctl daemon-reload
systemctl enable actraild.service
systemctl restart actraild.service
"${MODULE_DIR}/wait-service-active.sh" actraild.service

echo "installed auto config: host eBPF resolves at daemon startup;"
echo "workload seccomp-notify resolves independently at launch"
if [[ -n "$OTEL_ENDPOINT" ]]; then
    echo "exporters=otel-jsonl,otel-http"
    echo "otel_endpoint=$OTEL_ENDPOINT"
    echo "otel_attribute_mode=$OTEL_ATTRIBUTE_MODE"
else
    echo "exporters=otel-jsonl"
fi
