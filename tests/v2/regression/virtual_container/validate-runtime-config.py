#!/usr/bin/env python3
"""Validate filesystem references in a selected Kata runtime configuration."""

from __future__ import annotations

import argparse
import fnmatch
import os
import sys
from pathlib import Path
from typing import Any, Dict, Optional

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.kata_runtime import (  # noqa: E402
    load_hypervisor_table,
    supported_backends,
)

REQUIRED_VIRTIO_FS_KERNEL_CONFIG = (
    "CONFIG_FUSE_FS=y",
    "CONFIG_VIRTIO_FS=y",
    "CONFIG_VIRTIO_MMIO=y",
    "CONFIG_VSOCKETS=y",
    "CONFIG_VIRTIO_VSOCKETS=y",
    "CONFIG_VIRTIO_VSOCKETS_COMMON=y",
)
REQUIRED_EBPF_KERNEL_CONFIG = (
    "CONFIG_BPF=y",
    "CONFIG_BPF_SYSCALL=y",
    "CONFIG_BPF_JIT=y",
    "CONFIG_BPF_EVENTS=y",
    "CONFIG_DEBUG_INFO_BTF=y",
    "CONFIG_FTRACE=y",
    "CONFIG_FTRACE_SYSCALLS=y",
    "CONFIG_KPROBES=y",
    "CONFIG_KPROBE_EVENTS=y",
    "CONFIG_PERF_EVENTS=y",
    "CONFIG_TRACEPOINTS=y",
    "CONFIG_TRACING=y",
    "CONFIG_UPROBES=y",
    "CONFIG_UPROBE_EVENTS=y",
)


def fail(message: str) -> None:
    print(f"runtime config invalid: {message}", file=sys.stderr)


def nonempty_path(table: Dict[str, Any], key: str) -> Optional[Path]:
    value = table.get(key)
    if not isinstance(value, str) or not value.strip():
        return None
    return Path(value)


def discover_kernel_config(kernel: Path) -> Optional[Path]:
    candidates = [Path(f"{kernel}.config")]
    try:
        resolved = kernel.resolve(strict=True)
    except OSError:
        resolved = kernel
    for prefix in ("vmlinux-", "vmlinuz-"):
        if resolved.name.startswith(prefix):
            candidates.append(
                resolved.with_name(f"config-{resolved.name[len(prefix):]}")
            )
            break
    for candidate in candidates:
        if candidate.is_file():
            return candidate
    return None


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument(
        "--backend",
        required=True,
        choices=supported_backends(),
    )
    parser.add_argument(
        "--kernel-config",
        type=Path,
        help="resolved guest-kernel .config (default: KERNEL.config if present)",
    )
    parser.add_argument(
        "--require-kernel-config",
        action="store_true",
        help="reject a virtio-fs configuration without a readable .config",
    )
    parser.add_argument(
        "--require-ebpf",
        action="store_true",
        help="require the guest-kernel BTF, tracing and eBPF configuration",
    )
    parser.add_argument("config", type=Path)
    args = parser.parse_args()
    try:
        table = load_hypervisor_table(args.config, args.backend)
    except (OSError, KeyError, TypeError, ValueError) as error:
        fail(f"cannot read {args.config}: {error}")
        return 1

    errors = []  # type: list[str]
    vmm_path = nonempty_path(table, "path")
    kernel_path = nonempty_path(table, "kernel")
    image_path = nonempty_path(table, "image")
    initrd_path = nonempty_path(table, "initrd")
    virtiofsd_path = nonempty_path(table, "virtio_fs_daemon")

    if vmm_path is None:
        errors.append("path is missing")
    elif not vmm_path.is_file():
        errors.append(f"VMM path does not exist: {vmm_path}")
    elif not os.access(vmm_path, os.X_OK):
        errors.append(f"VMM path is not executable: {vmm_path}")

    if kernel_path is None:
        errors.append("kernel is missing")
    elif not kernel_path.is_file():
        errors.append(f"kernel does not exist: {kernel_path}")

    rootfs_paths = [
        (name, path)
        for name, path in (("image", image_path), ("initrd", initrd_path))
        if path is not None
    ]
    if not rootfs_paths:
        errors.append("neither image nor initrd is configured")
    for name, path in rootfs_paths:
        if not path.is_file():
            errors.append(f"{name} does not exist: {path}")

    allowed_paths = table.get("valid_hypervisor_paths", [])
    if (
        vmm_path is not None
        and isinstance(allowed_paths, list)
        and allowed_paths
        and not any(
            isinstance(pattern, str)
            and fnmatch.fnmatch(str(vmm_path), pattern)
            for pattern in allowed_paths
        )
    ):
        errors.append(
            f"VMM path is rejected by valid_hypervisor_paths: {vmm_path}"
        )

    shared_fs = table.get("shared_fs")
    uses_virtio_fs = isinstance(shared_fs, str) and shared_fs.startswith(
        "virtio-fs"
    )
    checked_kernel_config: Optional[Path] = None
    configured_lines = None  # type: Optional[set[str]]
    kernel_config = args.kernel_config
    if kernel_config is None and kernel_path is not None:
        kernel_config = discover_kernel_config(kernel_path)
    must_check_kernel = (
        args.require_kernel_config
        or args.require_ebpf
        or (uses_virtio_fs and kernel_config is not None)
    )
    if must_check_kernel:
        if kernel_config is None:
            errors.append(
                "guest kernel .config is required to verify requested capabilities"
            )
        elif not kernel_config.is_file():
            errors.append(f"guest kernel config does not exist: {kernel_config}")
        else:
            checked_kernel_config = kernel_config
            try:
                configured_lines = set(
                    kernel_config.read_text(encoding="utf-8").splitlines()
                )
            except OSError as error:
                errors.append(
                    f"cannot read guest kernel config {kernel_config}: {error}"
                )

    if uses_virtio_fs:
        if virtiofsd_path is None:
            errors.append("virtio_fs_daemon is missing for virtio-fs")
        elif not virtiofsd_path.is_file():
            errors.append(f"virtiofsd does not exist: {virtiofsd_path}")
        elif not os.access(virtiofsd_path, os.X_OK):
            errors.append(f"virtiofsd is not executable: {virtiofsd_path}")

        allowed_virtiofsd_paths = table.get(
            "valid_virtio_fs_daemon_paths",
            [],
        )
        if (
            virtiofsd_path is not None
            and isinstance(allowed_virtiofsd_paths, list)
            and allowed_virtiofsd_paths
            and not any(
                isinstance(pattern, str)
                and fnmatch.fnmatch(str(virtiofsd_path), pattern)
                for pattern in allowed_virtiofsd_paths
            )
        ):
            errors.append(
                "virtiofsd is rejected by valid_virtio_fs_daemon_paths: "
                f"{virtiofsd_path}"
            )

        if configured_lines is not None:
            for expected_line in REQUIRED_VIRTIO_FS_KERNEL_CONFIG:
                if expected_line not in configured_lines:
                    errors.append(
                        "guest kernel config is missing: "
                        f"{expected_line} ({kernel_config})"
                    )

    if args.require_ebpf and configured_lines is not None:
        for expected_line in REQUIRED_EBPF_KERNEL_CONFIG:
            if expected_line not in configured_lines:
                errors.append(
                    "guest eBPF kernel config is missing: "
                    f"{expected_line} ({kernel_config})"
                )

    if errors:
        for error in errors:
            fail(error)
        return 1

    rootfs_kinds = ",".join(name for name, _ in rootfs_paths)
    print(
        "runtime_config_valid "
        f"backend={args.backend} config={args.config} rootfs={rootfs_kinds} "
        f"kernel_config={checked_kernel_config or 'unchecked'}"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
