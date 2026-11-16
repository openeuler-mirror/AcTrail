from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class ContainerManagerDefaults:
    image_version: str = "latest"
    force_rebuild: bool = False
    command: tuple[str, ...] = ("tail", "-f", "/dev/null")
    force_overwrite: bool = False
    dismiss_on_exit: bool = True
    ready_timeout_seconds: float = 15.0
    ready_poll_seconds: float = 0.05


CONTAINER_DEFAULTS = ContainerManagerDefaults()
