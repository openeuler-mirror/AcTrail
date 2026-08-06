#!/bin/sh
# Guest-side capability precheck for the V2 virtual-container case.
set -eu

SCRIPT_DIR="$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)"
. "$SCRIPT_DIR/common.sh"

actrail_need_binaries
mount_guest_observation_fs

echo "== AcTrail Kata guest precheck =="
echo "arch=$(uname -m)"
echo "kernel=$(uname -r)"

if [ -n "${EXPECTED_ARCH:-}" ] && [ "$(uname -m)" != "$EXPECTED_ARCH" ]; then
  fail "unexpected guest arch: got $(uname -m), expected $EXPECTED_ARCH"
fi

if [ -e /sys/kernel/btf/vmlinux ]; then
  echo "BTF=YES"
else
  echo "BTF=NO"
  [ "${REQUIRE_EBPF:-0}" = 0 ] || fail "required BTF file is missing"
fi

if ls /sys/kernel/tracing >/dev/null 2>&1; then
  echo "tracefs=YES"
else
  echo "tracefs=NO"
  [ "${REQUIRE_EBPF:-0}" = 0 ] || fail "tracefs unavailable"
fi

if ls /sys/fs/bpf >/dev/null 2>&1; then
  echo "bpffs=YES"
else
  echo "bpffs=NO"
  [ "${REQUIRE_EBPF:-0}" = 0 ] || fail "bpffs unavailable"
fi

config_file=""
if [ -r /proc/config.gz ]; then
  config_file=/proc/config.gz
elif [ -r "/boot/config-$(uname -r)" ]; then
  config_file="/boot/config-$(uname -r)"
fi

if [ -n "$config_file" ]; then
  echo "kernel_config=$config_file"
  if [ "$config_file" = /proc/config.gz ]; then
    zcat "$config_file" | grep -E 'CONFIG_BPF=|CONFIG_BPF_SYSCALL=|CONFIG_BPF_JIT=|CONFIG_DEBUG_INFO_BTF=|CONFIG_FTRACE_SYSCALLS=' || true
  else
    grep -E 'CONFIG_BPF=|CONFIG_BPF_SYSCALL=|CONFIG_BPF_JIT=|CONFIG_DEBUG_INFO_BTF=|CONFIG_FTRACE_SYSCALLS=' "$config_file" || true
  fi
else
  echo "kernel_config=UNAVAILABLE"
fi

if [ -d /sys/class/dmi/id ]; then
  echo "dmi_dir=YES"
  ls /sys/class/dmi/id 2>/dev/null | head -20 || true
else
  echo "dmi_dir=NO"
fi

echo "cgroup_self=$(tr '\n' ';' </proc/self/cgroup)"
"$(openssl_bin)" version
"$ACTRAILCTL" --config "${ACTRAIL_BUNDLE}/guest-combo.conf" probe \
  --host-ebpf auto --seccomp-notify disabled --skip-daemon

echo "GUEST_PRECHECK_OK"
