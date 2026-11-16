from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass

from server_core.application import LocalMaaSApplication
from server_core.api_endpoints import ProtocolEndpoint, RestApi
from server_core.config import ServerCoreConfig


class ConnectionStartupError(RuntimeError):
    """Raised when a configured connection cannot start."""


@dataclass(frozen=True, slots=True)
class ConnectionDescription:
    service: str
    scheme: str
    host: str
    port: int
    origin: str
    endpoints: tuple[ProtocolEndpoint, ...]
    rest_apis: tuple[RestApi, ...]
    ca_cert_file: str | None = None


class ConnectionServer(ABC):
    @property
    @abstractmethod
    def config(self) -> ServerCoreConfig:
        raise NotImplementedError

    @property
    @abstractmethod
    def application(self) -> LocalMaaSApplication:
        raise NotImplementedError

    @property
    @abstractmethod
    def scheme(self) -> str:
        raise NotImplementedError

    @property
    @abstractmethod
    def origin(self) -> str:
        raise NotImplementedError

    @property
    @abstractmethod
    def description(self) -> ConnectionDescription:
        raise NotImplementedError

    @abstractmethod
    def serve_forever(self) -> None:
        raise NotImplementedError

    @abstractmethod
    def shutdown(self) -> None:
        raise NotImplementedError

    @abstractmethod
    def close(self) -> None:
        raise NotImplementedError
