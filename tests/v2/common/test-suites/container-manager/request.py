from __future__ import annotations

from dataclasses import dataclass

from image import ContainerImage


@dataclass(frozen=True)
class ContainerRequest:
    image: ContainerImage
    name: str
    command: tuple[str, ...] = ("tail", "-f", "/dev/null")
    labels: tuple[str, ...] = ()
    volumes: tuple[str, ...] = ()
    security_options: tuple[str, ...] = ()
    user: str | None = None
    network: str | None = None
    pid: str | None = None
    force_overwrite: bool = False
    dismiss_on_exit: bool = True
    ready_timeout_seconds: float = 15.0

    def __post_init__(self) -> None:
        if not self.name:
            raise ValueError("container name must not be empty")
        if not self.command:
            raise ValueError("container command must not be empty")
        if self.ready_timeout_seconds <= 0:
            raise ValueError("ready_timeout_seconds must be positive")
