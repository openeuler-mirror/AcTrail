#!/usr/bin/env bash
# Verify fail-closed validation of Kata VMM, kernel and guest rootfs references.
set -euo pipefail

ROOT_DIR="${ACTRAIL_REPO_ROOT:-$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../../../.." && pwd)}"
VALIDATOR="$ROOT_DIR/tests/v2/regression/virtual_container/validate-runtime-config.py"

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

TEMP_ROOT="${TMPDIR:-/tmp}"
WORK_DIR="$(mktemp -d "${TEMP_ROOT%/}/actrail-runtime-config-paths.XXXXXX")"
trap 'rm -rf -- "$WORK_DIR"' EXIT
VMM="$WORK_DIR/stratovirt"
KERNEL="$WORK_DIR/kernel"
IMAGE="$WORK_DIR/guest.img"
VIRTIOFSD="$WORK_DIR/virtiofsd"
VALID_CONFIG="$WORK_DIR/valid.toml"
INVALID_CONFIG="$WORK_DIR/invalid.toml"
DISCOVERED_CONFIG="$WORK_DIR/discovered.toml"

printf '#!/bin/sh\nexit 0\n' >"$VMM"
printf '#!/bin/sh\nexit 0\n' >"$VIRTIOFSD"
chmod +x "$VMM" "$VIRTIOFSD"
touch "$KERNEL" "$IMAGE"
cat >"$KERNEL.config" <<'EOF'
CONFIG_FUSE_FS=y
CONFIG_VIRTIO_FS=y
CONFIG_VIRTIO_MMIO=y
CONFIG_VSOCKETS=y
CONFIG_VIRTIO_VSOCKETS=y
CONFIG_VIRTIO_VSOCKETS_COMMON=y
EOF

cat >"$VALID_CONFIG" <<EOF
[hypervisor.stratovirt]
path = "$VMM"
kernel = "$KERNEL"
image = "$IMAGE"
valid_hypervisor_paths = ["$WORK_DIR/*"]
shared_fs = "virtio-fs"
virtio_fs_daemon = "$VIRTIOFSD"
valid_virtio_fs_daemon_paths = ["$WORK_DIR/*"]
EOF

"$VALIDATOR" --backend stratovirt "$VALID_CONFIG" >/dev/null \
  || fail "valid runtime filesystem references were rejected"

set +e
missing_ebpf_output="$(
  "$VALIDATOR" --backend stratovirt --require-ebpf "$VALID_CONFIG" 2>&1
)"
missing_ebpf_rc=$?
set -e
[[ "$missing_ebpf_rc" -ne 0 ]] \
  || fail "eBPF validation accepted a kernel config without BPF syscall support"
grep -Fq "guest eBPF kernel config is missing: CONFIG_BPF_SYSCALL=y" \
  <<<"$missing_ebpf_output" \
  || fail "missing eBPF capability diagnostic was not emitted"

cat >>"$KERNEL.config" <<'EOF'
CONFIG_BPF=y
CONFIG_BPF_SYSCALL=y
CONFIG_BPF_JIT=y
CONFIG_BPF_EVENTS=y
CONFIG_DEBUG_INFO_BTF=y
CONFIG_FTRACE=y
CONFIG_FTRACE_SYSCALLS=y
CONFIG_KPROBES=y
CONFIG_KPROBE_EVENTS=y
CONFIG_PERF_EVENTS=y
CONFIG_TRACEPOINTS=y
CONFIG_TRACING=y
CONFIG_UPROBES=y
CONFIG_UPROBE_EVENTS=y
EOF

"$VALIDATOR" --backend stratovirt --require-ebpf "$VALID_CONFIG" >/dev/null \
  || fail "valid eBPF kernel configuration was rejected"

# Official Kata static archives expose a friendly kernel symlink while keeping
# the matching configuration beside its versioned target.
KATA_KERNEL_DIR="$WORK_DIR/kata-kernels"
KATA_KERNEL_TARGET="$KATA_KERNEL_DIR/vmlinux-6.18.35-197-debug"
KATA_KERNEL_LINK="$KATA_KERNEL_DIR/vmlinux-debug.container"
KATA_KERNEL_CONFIG="$KATA_KERNEL_DIR/config-6.18.35-197-debug"
mkdir -p "$KATA_KERNEL_DIR"
touch "$KATA_KERNEL_TARGET"
ln -s "$(basename "$KATA_KERNEL_TARGET")" "$KATA_KERNEL_LINK"
cp "$KERNEL.config" "$KATA_KERNEL_CONFIG"
KATA_KERNEL_CONFIG_REAL="$(realpath "$KATA_KERNEL_CONFIG")"
cat >"$DISCOVERED_CONFIG" <<EOF
[hypervisor.stratovirt]
path = "$VMM"
kernel = "$KATA_KERNEL_LINK"
image = "$IMAGE"
valid_hypervisor_paths = ["$WORK_DIR/*"]
shared_fs = "virtio-fs"
virtio_fs_daemon = "$VIRTIOFSD"
valid_virtio_fs_daemon_paths = ["$WORK_DIR/*"]
EOF

discovered_output="$(
  "$VALIDATOR" \
    --backend stratovirt \
    --require-kernel-config \
    --require-ebpf \
    "$DISCOVERED_CONFIG"
)" || fail "official Kata kernel config layout was not discovered"
grep -Fq "kernel_config=$KATA_KERNEL_CONFIG_REAL" <<<"$discovered_output" \
  || fail "validator reported the wrong discovered Kata kernel config"

grep -Fvx 'CONFIG_VIRTIO_FS=y' "$KERNEL.config" >"$KERNEL.config.tmp"
mv "$KERNEL.config.tmp" "$KERNEL.config"
set +e
missing_capability_output="$(
  "$VALIDATOR" --backend stratovirt "$VALID_CONFIG" 2>&1
)"
missing_capability_rc=$?
set -e
[[ "$missing_capability_rc" -ne 0 ]] \
  || fail "virtio-fs runtime accepted a kernel config without CONFIG_VIRTIO_FS"
grep -Fq "guest kernel config is missing: CONFIG_VIRTIO_FS=y" \
  <<<"$missing_capability_output" \
  || fail "missing CONFIG_VIRTIO_FS diagnostic was not emitted"
printf '%s\n' 'CONFIG_VIRTIO_FS=y' >>"$KERNEL.config"

cat >"$INVALID_CONFIG" <<EOF
[hypervisor.stratovirt]
path = "$WORK_DIR/missing-vmm"
kernel = "$WORK_DIR/missing-kernel"
valid_hypervisor_paths = ["/usr/bin/*"]
shared_fs = "virtio-fs"
virtio_fs_daemon = "$WORK_DIR/missing-virtiofsd"
valid_virtio_fs_daemon_paths = ["/usr/libexec/*"]
EOF

set +e
invalid_output="$(
  "$VALIDATOR" --backend stratovirt "$INVALID_CONFIG" 2>&1
)"
invalid_rc=$?
set -e
[[ "$invalid_rc" -ne 0 ]] \
  || fail "missing runtime filesystem references were accepted"
grep -Fq "VMM path does not exist" <<<"$invalid_output" \
  || fail "missing VMM path diagnostic was not emitted"
grep -Fq "kernel does not exist" <<<"$invalid_output" \
  || fail "missing kernel diagnostic was not emitted"
grep -Fq "neither image nor initrd is configured" <<<"$invalid_output" \
  || fail "missing guest rootfs diagnostic was not emitted"
grep -Fq "rejected by valid_hypervisor_paths" <<<"$invalid_output" \
  || fail "VMM allowlist diagnostic was not emitted"
grep -Fq "virtiofsd does not exist" <<<"$invalid_output" \
  || fail "missing virtiofsd diagnostic was not emitted"
grep -Fq "rejected by valid_virtio_fs_daemon_paths" <<<"$invalid_output" \
  || fail "virtiofsd allowlist diagnostic was not emitted"

echo "RUNTIME_CONFIG_PATHS_TEST_OK"
