"""Host preparation and preflight defaults for the virtual-container case."""

from __future__ import annotations

import shlex
from dataclasses import dataclass, field, fields
from pathlib import Path


@dataclass(frozen=True)
class HostConstants:
    HOST_REPO_ROOT: Path = field(
        default_factory=lambda: Path(__file__).resolve().parents[4]
    )
    BUNDLE_SUBDIRECTORY: str = ".actrail-guest-bundle"
    BUNDLE_SYSROOT: str = "/"
    BUNDLE_TARGET_GLIBC: str = ""
    BUILD_PROFILE: str = "release"
    ACTRAIL_BUILD: int = 1
    COPY_OPENSSL: int = 1
    EBPF_TRANSPORT: str = "auto"
    BUNDLE_OPENSSL: str = ""
    BUNDLE_MAX_SYMLINK_DEPTH: int = 40
    CARGO_TARGET_SUBDIRECTORY: str = "target"
    BACKEND: str = "stratovirt"
    RUNTIME_CONFIG_PATH: str = ""
    CGROUP_ROOT: str = "/sys/fs/cgroup"
    KATA_CONFIG_DIRS: str = (
        "/opt/kata/share/defaults/kata-containers:"
        "/usr/share/defaults/kata-containers:/etc/kata-containers"
    )
    KATA_PREFERRED_SHIM_BINARY: str = "containerd-shim-kata332-v2"
    KATA_PREFERRED_CTR_RUNTIME: str = "io.containerd.kata332.v2"
    KATA_DEFAULT_CTR_RUNTIME: str = "io.containerd.kata.v2"
    KATA_STRATOVIRT_CONFIG_NAME: str = "configuration-stratovirt.toml"
    KATA_CLOUD_HYPERVISOR_CONFIG_NAME: str = "configuration-clh.toml"

    def shell_defaults(self) -> str:
        lines = []
        for definition in fields(self):
            name = definition.name
            value = shlex.quote(str(getattr(self, name)))
            lines.append(f'if [ -z "${{{name}:-}}" ]; then {name}={value}; fi')
        return "\n".join(lines)


CASE_CONSTANTS = HostConstants()


if __name__ == "__main__":
    print(CASE_CONSTANTS.shell_defaults())
