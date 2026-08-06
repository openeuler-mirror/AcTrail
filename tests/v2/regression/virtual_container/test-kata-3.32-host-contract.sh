#!/usr/bin/env bash
# Static contract for the pinned, side-by-side Kata 3.32 ARM64 installation.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
INSTALLER="$ROOT_DIR/deploy/virtual-container/host/install-kata-3.32.sh"
CONFIGURATOR="$ROOT_DIR/deploy/virtual-container/host/prepare-stratovirt-config.py"
PREPARER="$ROOT_DIR/deploy/virtual-container/host/prepare-v2-test-artifacts.py"
PREPARER_MODULE="$ROOT_DIR/deploy/virtual-container/host/v2_artifacts.py"
V2_RUNNER="$ROOT_DIR/deploy/virtual-container/host/run-v2-tests.sh"
BUILDER="$ROOT_DIR/deploy/virtual-container/guest/build-openeuler-image.sh"
DEPLOY_README="$ROOT_DIR/deploy/virtual-container/README.md"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

bash -n "$INSTALLER" || fail "Kata 3.32 installer has invalid shell syntax"
bash -n "$V2_RUNNER" || fail "V2 one-command runner has invalid shell syntax"
"$PREPARER" --help | grep -Fq -- '--otel-endpoint' \
  || fail "V2 artifact preparer does not require a Guest Collector endpoint"
python3 - "$CONFIGURATOR" "$PREPARER" "$PREPARER_MODULE" <<'PY' \
  || fail "virtual-container host tools have invalid Python syntax"
from pathlib import Path
import sys

for name in sys.argv[1:]:
    source = Path(name).read_text(encoding="utf-8")
    compile(source, name, "exec")
PY
python3 - "$DEPLOY_README" <<'PY' \
  || fail "deployment README does not install release before preparing artifacts"
from pathlib import Path
import sys

source = Path(sys.argv[1]).read_text(encoding="utf-8")
release = source.find("bash scripts/install-release.sh")
prepare = source.find("prepare-v2-test-artifacts.py")
if release < 0 or prepare < 0 or release > prepare:
    raise SystemExit(1)
PY

grep -Fqx 'VERSION="3.32.0"' "$INSTALLER" \
  || fail "installer does not pin Kata 3.32.0"
grep -Fqx \
  'ARCHIVE_SHA256="8736c054d9223974735394f822000823baef509e1c33405ec798240fa9b6e4b5"' \
  "$INSTALLER" \
  || fail "installer does not pin the official ARM64 archive digest"
grep -Fqx 'PREFIX="/opt/kata-$VERSION"' "$INSTALLER" \
  || fail "installer does not preserve a versioned side-by-side prefix"
grep -Fq '/usr/local/bin/containerd-shim-kata-v2' "$INSTALLER" \
  || fail "installer does not activate shim-v2 through /usr/local"
grep -Fq 'VERSIONED_SHIM="containerd-shim-kata332-v2"' "$INSTALLER" \
  || fail "installer does not provide an unambiguous Kata 3.32 shim alias"
grep -Fq 'VERSIONED_RUNTIME="io.containerd.kata332.v2"' "$INSTALLER" \
  || fail "installer does not publish the versioned containerd runtime name"
grep -Fq 'configuration-clh.toml' "$INSTALLER" \
  || fail "installer does not validate the Cloud Hypervisor configuration"
grep -Fq 'bin/cloud-hypervisor' "$INSTALLER" \
  || fail "installer does not validate the Cloud Hypervisor binary"
if grep -Eq "(^|[[:space:]])(mv|install|ln)[[:space:]].*[/]usr/bin" "$INSTALLER"; then
  fail "installer must not overwrite distro-owned /usr/bin Kata binaries"
fi
grep -Fq 'existing prefix has the wrong Kata version' "$INSTALLER" \
  || fail "installer does not protect an existing versioned prefix"
grep -Fq 'EXPECTED_KATA_VERSION = "3.32.0"' "$CONFIGURATOR" \
  || fail "configurator does not require the matching Kata release"
grep -Fq -- '--image-config-path' "$CONFIGURATOR" \
  || fail "configurator cannot publish a config from a staging image"
for setting in \
  valid_hypervisor_paths \
  valid_virtio_fs_daemon_paths \
  default_vcpus \
  debug_console_enabled; do
  grep -Fq "$setting" "$CONFIGURATOR" \
    || fail "configurator does not control $setting"
done
grep -Fq -- '--require-agent-policy' "$BUILDER" \
  || fail "openEuler guest builder does not require the 3.32 policy asset"
grep -Fq 'sbin/init' "$BUILDER" \
  || fail "openEuler guest builder does not extract the 3.32 agent"
grep -Fq 'artifact_cache=hit' "$PREPARER_MODULE" \
  || fail "V2 artifact preparer does not expose cache hits"
grep -Fq 'os.replace(staging, final)' "$PREPARER_MODULE" \
  || fail "V2 artifact preparer does not atomically publish staging"
grep -Fq -- '--case virtual_container_xiaoo_concurrency' "$V2_RUNNER" \
  || fail "V2 one-command runner does not select both virtual-container cases"

echo "KATA_3_32_HOST_CONTRACT_OK"
