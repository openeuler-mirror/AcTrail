#!/usr/bin/env bash
# Build the kernel module under CARGO_TARGET_DIR/kernel/<kernel-release>.
set -euo pipefail
source_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
repo_root="$(cd -- "$source_dir/../../.." && pwd)"
target_dir="${CARGO_TARGET_DIR:-$repo_root/target}"
if [[ "$target_dir" != /* ]]; then
  target_dir="$repo_root/$target_dir"
fi
kernel_release="$(uname -r)"
module_build_dir="$target_dir/kernel/$kernel_release"
kernel_build="${ACTRAIL_KERNEL_BUILD:-/lib/modules/$kernel_release/build}"
kernel_btf="${ACTRAIL_KERNEL_BTF:-/sys/kernel/btf/vmlinux}"

# Kbuild writes beside its module sources, so provide copies in the build directory.
mkdir -p "$module_build_dir"
cp -p "$source_dir/Makefile" "$source_dir/"*.c "$source_dir/"*.h "$module_build_dir/"
make -C "$kernel_build" M="$module_build_dir" modules
module_path="$module_build_dir/actrail_lsm_grey.ko"
pahole -J --btf_base "$kernel_btf" "$module_path"
"$kernel_build/tools/bpf/resolve_btfids/resolve_btfids" -b "$kernel_btf" "$module_path"
