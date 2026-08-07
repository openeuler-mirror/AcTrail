#!/usr/bin/env bash
# 虚拟容器测试环境预检：验收机必须通过必要能力门禁。
set -u

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
. "$ROOT_DIR/tests/v2/regression/virtual_container/runtime-backend.sh"
RUNTIME_CONFIG_VALIDATOR="$ROOT_DIR/tests/v2/regression/virtual_container/validate-runtime-config.py"
BACKEND="${BACKEND:-stratovirt}" # stratovirt|cloud-hypervisor|all
RUNTIME_CONFIG_PATH="${RUNTIME_CONFIG_PATH:-}"
CGROUP_ROOT="${CGROUP_ROOT:-/sys/fs/cgroup}"
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
check() { # $1 描述, $2 命令
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
    echo "PASS: 宿主 cgroup v2"
    pass=$((pass+1))
    return
  fi

  for controller in "${required_v1_controllers[@]}"; do
    [[ -d "$CGROUP_ROOT/$controller" ]] || missing+=("$controller")
  done
  if [[ "${#missing[@]}" -eq 0 ]]; then
    echo "PASS: 宿主 cgroup v1 必要控制器"
    pass=$((pass+1))
  else
    echo "FAIL: 宿主 cgroup v1/v2（缺少 v1 控制器: ${missing[*]}）"
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
check "/dev/kvm 存在且可读写"            'test -r /dev/kvm -a -w /dev/kvm'
case "$(uname -m)" in
  x86_64) check "x86 CPU 虚拟化标志"       'grep -Eq "vmx|svm" /proc/cpuinfo' ;;
  aarch64) check "ARM64 KVM 设备"          'test -c /dev/kvm -a -d /sys/module/kvm' ;;
  *) check "受支持的虚拟化架构"            'false' ;;
esac
check "containerd daemon 可连接"         'command -v ctr && ctr version'
check "kata-runtime 诊断工具可用"       'command -v kata-runtime'
check "Kata shim-v2 可用 ($KATA_SHIM_BINARY)" \
  "command -v '$KATA_SHIM_BINARY'"
check "Kata runtime/shim 版本一致"       'check_runtime_shim_version_match'
check "Kata runtime 配置校验器可用"     'test -x "$RUNTIME_CONFIG_VALIDATOR"'
case "$BACKEND" in
  stratovirt)
    check "StratoVirt 可用"              'command -v stratovirt'
    check_runtime_config stratovirt
    ;;
  cloud-hypervisor)
    check "Cloud Hypervisor 可用"        'command -v cloud-hypervisor'
    check_runtime_config cloud-hypervisor
    ;;
  all)
    check "StratoVirt 可用"              'command -v stratovirt'
    check_runtime_config stratovirt
    check "Cloud Hypervisor 可用"        'command -v cloud-hypervisor'
    check_runtime_config cloud-hypervisor
    ;;
esac
check_host_cgroup

echo
echo "== 版本冻结记录(写入验收报告)=="
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
  echo "-- $c"; eval "$c" 2>/dev/null | head -3 || echo "(不可用)"
done

echo
echo "preflight: pass=$pass fail=$fail"
test "$fail" -eq 0
