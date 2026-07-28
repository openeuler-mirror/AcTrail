#!/usr/bin/env bash
# Install the host side of the four-way Docker permission auto-selection.
#
# Usage:
#   sudo deploy/container-auto/install-host.sh [BIN_DIR]
set -euo pipefail

if [[ ${EUID} -ne 0 ]]; then
    echo "must run as root (sudo)" >&2
    exit 1
fi

MODULE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${MODULE_DIR}/../.." && pwd)"
BIN_DIR="${1:-${REPO_ROOT}/target/release}"
PLUGIN_SOURCE_DIR="${REPO_ROOT}/examples/plugins/builtin/otel-jsonl"
PLUGIN_INSTALL_DIR="/etc/actrail/plugins/otel-jsonl"
PROBE_LIB="libactrail_tls_payload_probe_sync.so"

for f in actraild actrailctl actrailviewer "${PROBE_LIB}"; do
    if [[ ! -f "${BIN_DIR}/${f}" ]]; then
        echo "missing ${BIN_DIR}/${f} — build first" >&2
        exit 1
    fi
done

for f in \
    otel-jsonl.plugin.toml \
    otel-jsonl.config.toml \
    otel-jsonl.config.v1.schema.json; do
    if [[ ! -f "${PLUGIN_SOURCE_DIR}/${f}" ]]; then
        echo "missing ${PLUGIN_SOURCE_DIR}/${f}" >&2
        exit 1
    fi
done

install -d -m 0755 -o root -g root /run/actrail
install -d -m 0750 -o root -g root /var/lib/actrail
install -d -m 0750 -o root -g root /var/lib/actrail/export
install -d -m 0750 -o root -g root /var/log/actrail
install -d -m 0755 -o root -g root /etc/actrail
install -d -m 0755 -o root -g root "${PLUGIN_INSTALL_DIR}"

install -m 0755 "${BIN_DIR}/actraild" /usr/local/bin/actraild
install -m 0755 "${BIN_DIR}/actrailctl" /usr/local/bin/actrailctl
install -m 0755 "${BIN_DIR}/actrailviewer" /usr/local/bin/actrailviewer
install -m 0755 "${BIN_DIR}/${PROBE_LIB}" "/usr/local/bin/${PROBE_LIB}"

install -m 0644 "${MODULE_DIR}/container-auto.conf" \
    /etc/actrail/container-auto.conf
install -m 0644 \
    "${PLUGIN_SOURCE_DIR}/otel-jsonl.plugin.toml" \
    "${PLUGIN_SOURCE_DIR}/otel-jsonl.config.toml" \
    "${PLUGIN_SOURCE_DIR}/otel-jsonl.config.v1.schema.json" \
    "${PLUGIN_INSTALL_DIR}/"
install -m 0644 "${MODULE_DIR}/actraild.service" \
    /etc/systemd/system/actraild.service

systemctl daemon-reload
systemctl enable actraild.service
systemctl restart actraild.service

echo "installed auto config: host eBPF resolves at daemon startup;"
echo "workload seccomp-notify resolves independently at launch"
