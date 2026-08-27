from __future__ import annotations

import ast
import re
from pathlib import Path
from collections.abc import Iterable
from typing import Any

from .backend import kata_backend

try:
    import tomllib
except ModuleNotFoundError:
    tomllib = None


REQUIRED_VIRTIO_FS_KERNEL_CONFIG = (
    "CONFIG_FUSE_FS=y",
    "CONFIG_VIRTIO_FS=y",
    "CONFIG_VIRTIO_MMIO=y",
    "CONFIG_VSOCKETS=y",
    "CONFIG_VIRTIO_VSOCKETS=y",
    "CONFIG_VIRTIO_VSOCKETS_COMMON=y",
)
REQUIRED_FIRECRACKER_KERNEL_CONFIG = (
    "CONFIG_VIRTIO_MMIO=y",
    "CONFIG_VIRTIO_BLK=y",
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


def load_hypervisor_table(config: Path, backend: str) -> dict[str, Any]:
    """Load the selected Kata hypervisor table on Python 3.10 and newer."""

    section = kata_backend(backend).toml_section
    table_name = section.removeprefix("hypervisor.")
    if tomllib is not None:
        with config.open("rb") as source:
            document = tomllib.load(source)
        table = document["hypervisor"][table_name]
        if not isinstance(table, dict):
            raise ValueError(f"[{section}] is not a table")
        return table

    assignments: dict[str, Any] = {}
    active = False
    pending = ""
    with config.open("r", encoding="utf-8") as source:
        for raw_line in source:
            line = raw_line.strip()
            if not line or line.startswith("#"):
                continue
            if line.startswith("[") and line.endswith("]"):
                if active:
                    break
                active = line == f"[{section}]"
                continue
            if not active:
                continue
            pending = f"{pending} {line}".strip()
            if pending.count("[") > pending.count("]"):
                continue
            match = re.match(r"^([A-Za-z0-9_]+)\s*=\s*(.+)$", pending)
            pending = ""
            if match is None:
                continue
            key, literal = match.groups()
            literal = re.sub(r"\s+#.*$", "", literal)
            try:
                assignments[key] = ast.literal_eval(literal)
            except (SyntaxError, ValueError):
                continue
    if not assignments:
        raise KeyError(section)
    return assignments


def runtime_path(config: Path, backend: str, key: str) -> Path:
    table = load_hypervisor_table(config, backend)
    value = table.get(key)
    if not isinstance(value, str) or not value:
        raise ValueError(
            f"runtime config {config} does not define "
            f"[{kata_backend(backend).toml_section}].{key}"
        )
    return Path(value)


def discover_kernel_config(kernel: Path) -> Path | None:
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
    return next((candidate for candidate in candidates if candidate.is_file()), None)


def missing_kernel_config_entries(
    configured_lines: Iterable[str],
    required_lines: Iterable[str],
) -> tuple[str, ...]:
    configured = frozenset(configured_lines)
    return tuple(line for line in required_lines if line not in configured)
