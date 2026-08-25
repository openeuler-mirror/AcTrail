from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.kata_runtime.runtime_config import (
    REQUIRED_EBPF_KERNEL_CONFIG,
    discover_kernel_config,
    missing_kernel_config_entries,
)


RELEASE_FILES = {
    "actraild_sha256": "actraild",
    "actrailctl_sha256": "actrailctl",
    "actrail_sb_sha256": "actrail-sb",
    "actrail_vsock_gateway_sha256": "actrail-vsock-gateway",
    "actrailviewer_sha256": "actrailviewer",
    "tls_probe_sha256": "libactrail_tls_payload_probe_sync.so",
}


@dataclass(frozen=True)
class PreparationInputs:
    repo: Path
    bin_dir: Path
    output_root: Path
    backend: str
    runtime: str
    kata_prefix: Path
    base_config_source: Path
    data_config_source: Path
    base_image_source: Path
    data_image_source: Path
    hypervisor: Path
    base_kernel: Path
    data_kernel: Path
    virtiofsd: Path | None
    xiaoo: Path | None
    workload_image: str
    workload_image_archive: Path | None
    image_pull_policy: str
    otel_endpoint: str | None
    socket_gid: int
    data_vcpus: int
    sandbox_observer: bool = False
    egress_mode: str = "network"
    tool_inputs: tuple[Path, ...] = ()

    @property
    def jailer(self) -> Path | None:
        if self.backend != "firecracker":
            return None
        return self.hypervisor.resolve().with_name("jailer").resolve()

    @property
    def data_kernel_config(self) -> Path | None:
        return discover_kernel_config(self.data_kernel)

    def validate(self) -> None:
        if self.backend not in {
            "stratovirt",
            "cloud-hypervisor",
            "firecracker",
        }:
            raise ValueError(f"unsupported artifact backend: {self.backend}")
        if not self.runtime:
            raise ValueError("containerd runtime must not be empty")
        if self.image_pull_policy not in {"never", "missing", "always"}:
            raise ValueError("image pull policy must be never, missing or always")
        if self.egress_mode not in {"network", "vsock-bridge"}:
            raise ValueError("egress mode must be network or vsock-bridge")
        if self.otel_endpoint is not None and not self.otel_endpoint:
            raise ValueError("Guest OTLP/HTTP endpoint must be omitted or non-empty")
        if self.egress_mode == "vsock-bridge" and self.otel_endpoint is None:
            raise ValueError("vsock-bridge egress requires a Guest OTLP/HTTP endpoint")
        if not 1 <= self.socket_gid <= 2_147_483_647:
            raise ValueError("socket GID must be between 1 and 2147483647")
        if self.data_vcpus < 2:
            raise ValueError("data vCPUs must be at least 2")
        if not isinstance(self.sandbox_observer, bool):
            raise ValueError("sandbox observer selection must be boolean")
        if self.backend == "firecracker" and not self.sandbox_observer:
            raise ValueError(
                "Firecracker artifacts require --with-sandbox-observer"
            )
        for name, path in (
            ("repository", self.repo),
            ("release directory", self.bin_dir),
            ("output root parent", self.output_root.parent),
        ):
            if not path.is_absolute():
                raise ValueError(f"{name} path must be absolute: {path}")
        for name, path in (
            ("base config source", self.base_config_source),
            ("data config source", self.data_config_source),
            ("base image source", self.base_image_source),
            ("data image source", self.data_image_source),
            ("hypervisor", self.hypervisor),
            ("base kernel", self.base_kernel),
            ("data kernel", self.data_kernel),
        ):
            if not path.is_absolute() or not path.is_file():
                raise ValueError(f"{name} must be an existing absolute file: {path}")
        if self.sandbox_observer:
            kernel_config = self.data_kernel_config
            if kernel_config is None:
                raise ValueError(
                    "sandbox observer data kernel config is missing for "
                    f"{self.data_kernel}; pass --data-kernel with a bootable "
                    "BTF/eBPF kernel and KERNEL.config or config-VERSION sidecar"
                )
            try:
                configured_lines = kernel_config.read_text(
                    encoding="utf-8"
                ).splitlines()
            except OSError as error:
                raise ValueError(
                    "sandbox observer data kernel config is unreadable: "
                    f"{kernel_config}: {error}"
                ) from error
            missing = missing_kernel_config_entries(
                configured_lines,
                REQUIRED_EBPF_KERNEL_CONFIG,
            )
            if missing:
                raise ValueError(
                    "sandbox observer data kernel config "
                    f"{kernel_config} is missing required eBPF capabilities: "
                    f"{', '.join(missing)}; pass --data-kernel with a bootable "
                    "BTF/eBPF kernel"
                )
        if not os.access(self.hypervisor, os.X_OK):
            raise ValueError(f"hypervisor must be executable: {self.hypervisor}")
        if self.jailer is not None:
            if not self.jailer.is_absolute() or not self.jailer.is_file():
                raise ValueError(
                    "Firecracker jailer must be an existing hypervisor sibling: "
                    f"{self.jailer}"
                )
            if not os.access(self.jailer, os.X_OK):
                raise ValueError(
                    f"Firecracker jailer must be executable: {self.jailer}"
                )
        if self.virtiofsd is None:
            if self.backend != "firecracker":
                raise ValueError(
                    f"virtiofsd is required for artifact backend: {self.backend}"
                )
        elif not self.virtiofsd.is_absolute() or not self.virtiofsd.is_file():
            raise ValueError(
                "virtiofsd must be an existing absolute file: "
                f"{self.virtiofsd}"
            )
        elif not os.access(self.virtiofsd, os.X_OK):
            raise ValueError(f"virtiofsd must be executable: {self.virtiofsd}")
        if self.xiaoo is not None:
            if not self.xiaoo.is_absolute() or not self.xiaoo.is_file():
                raise ValueError(
                    f"xiaoO must be an existing absolute file: {self.xiaoo}"
                )
            if not os.access(self.xiaoo, os.X_OK):
                raise ValueError(f"xiaoO must be executable: {self.xiaoo}")
            if (
                self.backend == "firecracker"
                and self.workload_image_archive is None
            ):
                raise ValueError(
                    "Firecracker xiaoO artifacts require "
                    "--workload-image-archive so xiaoO can be preinstalled"
                )
        if self.workload_image_archive is not None and not (
            self.workload_image_archive.is_absolute()
            and self.workload_image_archive.is_file()
        ):
            raise ValueError(
                "workload image archive must be an existing absolute file: "
                f"{self.workload_image_archive}"
            )
        for filename in RELEASE_FILES.values():
            path = self.bin_dir / filename
            if not path.is_file():
                raise ValueError(f"release artifact is missing: {path}")
