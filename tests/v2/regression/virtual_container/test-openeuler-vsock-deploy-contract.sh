#!/usr/bin/env bash
# Contract for the checkout-level openEuler VSOCK deployment entrypoint.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
DEPLOY="$ROOT_DIR/deploy/virtual-container/host/deploy-openeuler-vsock.sh"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

bash -n "$DEPLOY" || fail "openEuler VSOCK deploy script has invalid syntax"

help_output="$($DEPLOY --help)"
for text in \
  'local/kata/artifacts/<digest>' \
  '--rebuild-workload' \
  '--skip-workload-build' \
  'the clean source image is copied and never modified in place' \
  'injected automatically into content-addressed base/data output images' \
  'architecture-bound inputs that must be supplied offline' \
  'Kata containers/VMs created by --run-tests are intentionally removed'; do
  grep -Fq -- "$text" <<<"$help_output" \
    || fail "deployment help omits cache contract: $text"
done

dry_output="$($DEPLOY \
  --dry-run \
  --rebuild-workload \
  --run-tests \
  --package-manager dnf \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf \
  --xiaoo /fixtures/xiaoo)"

for text in \
  'dnf install -y podman socat' \
  'systemctl enable --now containerd' \
  '/usr/local/bin/containerd-shim-kata332-v2' \
  'modprobe vhost_vsock' \
  'systemctl enable --now actrail-vsock-host-stratovirt.service' \
  'scripts/install-release.sh' \
  'podman build --format oci' \
  'podman save --format oci-archive' \
  'ctr -n default images import' \
  'workload_image_cache=refreshed runtime=containerd' \
  'prepare-v2-test-artifacts.py' \
  '--otel-endpoint http://127.0.0.1:14318/v1/traces' \
  '--egress-mode vsock-bridge' \
  'ACTRAIL_SKIP_JAVA_AGENT_BUILD=1' \
  'run-v2-tests.sh --color never' \
  'ACTRAIL_OPENEULER_VSOCK_DEPLOY_READY'; do
  grep -Fq -- "$text" <<<"$dry_output" \
    || fail "dry-run deployment omits: $text"
done
grep -F 'ACTRAIL_SKIP_JAVA_AGENT_BUILD=1' <<<"$dry_output" \
  | grep -Fq 'run-v2-tests.sh --color never' \
  || fail "V2 runner does not inherit the release installer feature environment"

# The entrypoint targets an openEuler Guest and workload image, which are
# arguments; the host distribution is recorded, not gated.
if grep -Fq 'requires an openEuler host' "$DEPLOY"; then
  fail "deployment entrypoint still refuses every non-openEuler host"
fi
grep -Fq 'host_os_id=' "$DEPLOY" \
  || fail "deployment entrypoint does not record the host OS it ran on"

apt_output="$($DEPLOY \
  --dry-run \
  --package-manager apt-get \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf)"
grep -Fq -- 'apt-get install -y podman socat' <<<"$apt_output" \
  || fail "deployment entrypoint cannot install packages with apt-get"

skip_output="$($DEPLOY \
  --dry-run \
  --skip-packages \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf)"
if grep -Eq '(dnf|apt-get) install' <<<"$skip_output"; then
  fail "--skip-packages still installs packages"
fi

if $DEPLOY \
  --dry-run \
  --package-manager zypper \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf >/dev/null 2>&1; then
  fail "deployment entrypoint accepts an unsupported package manager"
fi

python3 - "$dry_output" <<'PY' \
  || fail "deployment does not build release before preparing artifacts"
import sys

output = sys.argv[1]
release = output.find("scripts/install-release.sh")
prepare = output.find("prepare-v2-test-artifacts.py")
if release < 0 or prepare < 0 or release > prepare:
    raise SystemExit(1)
PY

cloud_output="$($DEPLOY \
  --dry-run \
  --skip-packages \
  --skip-workload-build \
  --backend cloud-hypervisor \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf)"
grep -Fq 'configuration-clh.toml' <<<"$cloud_output" \
  || fail "Cloud Hypervisor deployment does not select its config"
grep -Fq \
  'systemctl enable --now actrail-vsock-host-cloud-hypervisor-reconcile.path' \
  <<<"$cloud_output" \
  || fail "Cloud Hypervisor deployment does not start reconcile"
if grep -Fq '+ modprobe vhost_vsock' <<<"$cloud_output"; then
  fail "Cloud Hypervisor deployment unnecessarily requires vhost_vsock"
fi

set +e
network_output="$($DEPLOY \
  --dry-run \
  --egress-mode network \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf 2>&1)"
network_rc=$?
conflict_output="$($DEPLOY \
  --dry-run \
  --skip-workload-build \
  --rebuild-workload \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf 2>&1)"
conflict_rc=$?
missing_xiaoo_output="$($DEPLOY \
  --dry-run \
  --run-tests \
  --base-image-source /fixtures/openEuler-kata.image \
  --data-kernel /fixtures/vmlinuz-btf 2>&1)"
missing_xiaoo_rc=$?
set -e

[[ "$network_rc" -ne 0 ]] \
  || fail "network mode accepted the VSOCK default endpoint"
grep -Fq 'network egress requires an explicit --otel-endpoint' \
  <<<"$network_output" \
  || fail "network endpoint rejection is unclear"
[[ "$conflict_rc" -ne 0 ]] \
  || fail "conflicting workload cache flags were accepted"
grep -Fq 'conflicts with --rebuild-workload' <<<"$conflict_output" \
  || fail "workload cache conflict diagnostic is unclear"
[[ "$missing_xiaoo_rc" -ne 0 ]] \
  || fail "--run-tests accepted a missing xiaoO asset"
grep -Fq -- '--xiaoo is required with --run-tests' <<<"$missing_xiaoo_output" \
  || fail "missing xiaoO diagnostic is unclear"

echo "OPENEULER_VSOCK_DEPLOY_CONTRACT_OK"
