#!/usr/bin/env bash
# Build module BTF and resolve kfunc IDs against the running kernel.
set -euo pipefail
cd "$(dirname "$0")"
kernel_build="${ACTRAIL_KERNEL_BUILD:-/lib/modules/$(uname -r)/build}"
kernel_btf="${ACTRAIL_KERNEL_BTF:-/sys/kernel/btf/vmlinux}"
make -C "$kernel_build" M="$PWD" modules
pahole -J --btf_base "$kernel_btf" actrail_lsm_grey.ko
"$kernel_build/tools/bpf/resolve_btfids/resolve_btfids" -b "$kernel_btf" actrail_lsm_grey.ko
