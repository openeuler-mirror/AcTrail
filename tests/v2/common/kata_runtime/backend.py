from __future__ import annotations

import re
from dataclasses import dataclass


@dataclass(frozen=True)
class KataBackend:
    name: str
    vmm_command: str
    toml_section: str
    default_config_name: str


_BACKENDS = {
    "stratovirt": KataBackend(
        name="stratovirt",
        vmm_command="stratovirt",
        toml_section="hypervisor.stratovirt",
        default_config_name="configuration-stratovirt.toml",
    ),
    "cloud-hypervisor": KataBackend(
        name="cloud-hypervisor",
        vmm_command="cloud-hypervisor",
        toml_section="hypervisor.clh",
        default_config_name="configuration-clh.toml",
    ),
}


def supported_backends() -> tuple[str, ...]:
    return tuple(_BACKENDS)


def kata_backend(name: str) -> KataBackend:
    try:
        return _BACKENDS[name]
    except KeyError as error:
        raise ValueError(f"unsupported Kata backend: {name}") from error


def shim_binary(runtime: str) -> str:
    match = re.fullmatch(r"io\.containerd\.([A-Za-z0-9_-]+)\.v2", runtime)
    if match is None:
        raise ValueError(
            "CTR_RUNTIME must have the form io.containerd.<handler>.v2"
        )
    return f"containerd-shim-{match.group(1)}-v2"
