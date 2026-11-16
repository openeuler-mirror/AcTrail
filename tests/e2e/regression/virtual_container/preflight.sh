#!/usr/bin/env bash
# Check required host capabilities for virtual-container tests.
set -u

. "$(dirname -- "${BASH_SOURCE[0]}")/runtime-backend.sh" || exit 1
ROOT_DIR="${ACTRAIL_REPO_ROOT:-$HOST_REPO_ROOT}"
RUNTIME_CONFIG_VALIDATOR="$ROOT_DIR/tests/e2e/regression/virtual_container/validate-runtime-config.py"
CTR_RUNTIME="${CTR_RUNTIME:-$(default_kata_ctr_runtime)}"
KATA_SHIM_BINARY="$(kata_shim_binary_for_runtime "$CTR_RUNTIME")" || exit 1

case "$BACKEND" in
  stratovirt|cloud-hypervisor|all) ;;
  *)
    echo "FAIL: unsupported BACKEND=$BACKEND (expected stratovirt, cloud-hypervisor or all)" >&2
    exit 1
    ;;
esac

pass=0; fail=0
check() { # $1 description, $2 command
  if eval "$2" >/dev/null 2>&1; then
    echo "PASS: $1"; pass=$((pass+1))
  else
    echo "FAIL: $1"; fail=$((fail+1))
  fi
}

check_runtime_config() {
  local backend="$1"
  local runtime_config=""
  local validation_output=""
  if runtime_config="$(resolve_kata_runtime_config "$backend" "$RUNTIME_CONFIG_PATH")"; then
    echo "PASS: Kata runtime config ($backend): $runtime_config"
    pass=$((pass+1))
    if validation_output="$(
      "$RUNTIME_CONFIG_VALIDATOR" \
        --backend "$backend" "$runtime_config" 2>&1
    )"; then
      echo "PASS: Kata runtime config filesystem references ($backend)"
      pass=$((pass+1))
    else
      printf '%s\n' "$validation_output" >&2
      echo "FAIL: Kata runtime config filesystem references ($backend)"
      fail=$((fail+1))
    fi
  else
    echo "FAIL: Kata runtime config ($backend)"
    fail=$((fail+1))
  fi
}

check_host_cgroup() {
  local controller=""
  local -a required_v1_controllers=(
    blkio
    cpu,cpuacct
    cpuset
    devices
    freezer
    hugetlb
    memory
    pids
  )
  local -a missing=()

  if [[ -f "$CGROUP_ROOT/cgroup.controllers" ]]; then
    echo "PASS: Host cgroup v2"
    pass=$((pass+1))
    return
  fi

  for controller in "${required_v1_controllers[@]}"; do
    [[ -d "$CGROUP_ROOT/$controller" ]] || missing+=("$controller")
  done
  if [[ "${#missing[@]}" -eq 0 ]]; then
    echo "PASS: Required host cgroup v1 controllers"
    pass=$((pass+1))
  else
    echo "FAIL: Host cgroup v1/v2 (missing v1 controllers: ${missing[*]})"
    fail=$((fail+1))
  fi
}

check_runtime_shim_version_match() {
  local runtime_version=""
  local shim_version=""

  runtime_version="$(
    kata-runtime version 2>/dev/null \
      | awk -F: '/^kata-runtime[[:space:]]*:/ {
          gsub(/[[:space:]]/, "", $2)
          print $2
          exit
        }'
  )"
  shim_version="$(
    "$KATA_SHIM_BINARY" --version 2>/dev/null \
      | sed -n 's/.*version: \([^, ]*\).*/\1/p' \
      | head -n 1
  )"
  [[ -n "$runtime_version" && "$runtime_version" == "$shim_version" ]]
}

echo "== AcTrail virtual-container preflight =="
echo "containerd_runtime=$CTR_RUNTIME"
check "/dev/kvm exists and is readable and writable"            'test -r /dev/kvm -a -w /dev/kvm'
case "$(uname -m)" in
  x86_64) check "x86 CPU virtualization flags"       'grep -Eq "vmx|svm" /proc/cpuinfo' ;;
  aarch64) check "ARM64 KVM device"          'test -c /dev/kvm -a -d /sys/module/kvm' ;;
  *) check "Supported virtualization architecture"            'false' ;;
esac
check "containerd daemon is reachable"         'command -v ctr && ctr version'
check "kata-runtime diagnostics are available"       'command -v kata-runtime'
check "Kata shim-v2 is available ($KATA_SHIM_BINARY)" \
  "command -v '$KATA_SHIM_BINARY'"
check "Kata runtime and shim versions match"       'check_runtime_shim_version_match'
check "Kata runtime config validator is available"     'test -x "$RUNTIME_CONFIG_VALIDATOR"'
case "$BACKEND" in
  stratovirt)
    check "StratoVirt is available"              'command -v stratovirt'
    check_runtime_config stratovirt
    ;;
  cloud-hypervisor)
    check "Cloud Hypervisor is available"        'command -v cloud-hypervisor'
    check_runtime_config cloud-hypervisor
    ;;
  all)
    check "StratoVirt is available"              'command -v stratovirt'
    check_runtime_config stratovirt
    check "Cloud Hypervisor is available"        'command -v cloud-hypervisor'
    check_runtime_config cloud-hypervisor
    ;;
esac
check_host_cgroup

echo
echo "== Version snapshot for the acceptance report=="
version_commands=(
  "ctr version"
  "kata-runtime version"
  "$KATA_SHIM_BINARY --version"
)
case "$BACKEND" in
  stratovirt) version_commands+=("stratovirt -V") ;;
  cloud-hypervisor) version_commands+=("cloud-hypervisor --version") ;;
  all) version_commands+=("stratovirt -V" "cloud-hypervisor --version") ;;
esac
version_commands+=("uname -rm")
for c in "${version_commands[@]}"; do
  echo "-- $c"; eval "$c" 2>/dev/null | head -3 || echo "(unavailable)"
done

echo
echo "preflight: pass=$pass fail=$fail"
test "$fail" -eq 0
