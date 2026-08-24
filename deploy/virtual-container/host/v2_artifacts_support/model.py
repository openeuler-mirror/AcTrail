from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path


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
    virtiofsd: Path
    xiaoo: Path | None
    workload_image: str
    workload_image_archive: Path | None
    image_pull_policy: str
    otel_endpoint: str | None
    socket_gid: int
    data_vcpus: int
    egress_mode: str = "network"
    tool_inputs: tuple[Path, ...] = ()

    def validate(self) -> None:
        if self.backend not in {"stratovirt", "cloud-hypervisor"}:
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
            ("virtiofsd", self.virtiofsd),
        ):
            if not path.is_absolute() or not path.is_file():
                raise ValueError(f"{name} must be an existing absolute file: {path}")
        for name, path in (
            ("hypervisor", self.hypervisor),
            ("virtiofsd", self.virtiofsd),
        ):
            if not os.access(path, os.X_OK):
                raise ValueError(f"{name} must be executable: {path}")
        if self.xiaoo is not None:
            if not self.xiaoo.is_absolute() or not self.xiaoo.is_file():
                raise ValueError(
                    f"xiaoO must be an existing absolute file: {self.xiaoo}"
                )
            if not os.access(self.xiaoo, os.X_OK):
                raise ValueError(f"xiaoO must be executable: {self.xiaoo}")
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
