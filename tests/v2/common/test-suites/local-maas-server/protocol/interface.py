from __future__ import annotations

from abc import ABC, abstractmethod
from dataclasses import dataclass
from typing import Any, Iterable

from scenario.model import ScenarioEmission, ScenarioRequest


class ProtocolRequestError(RuntimeError):
    def __init__(self, code: str, message: str):
        super().__init__(message)
        self.code = code


@dataclass(frozen=True, slots=True)
class ProtocolFrame:
    payload: bytes


@dataclass(frozen=True, slots=True)
class ProtocolResponse:
    status: int
    media_type: str
    body: bytes | None = None
    frames: Iterable[ProtocolFrame] | None = None

    def __post_init__(self) -> None:
        if (self.body is None) == (self.frames is None):
            raise ValueError("protocol response must contain body or frames")

    @property
    def is_stream(self) -> bool:
        return self.frames is not None


class ProtocolAdapter(ABC):
    @property
    @abstractmethod
    def name(self) -> str:
        raise NotImplementedError

    @property
    @abstractmethod
    def service_name(self) -> str:
        raise NotImplementedError

    @property
    @abstractmethod
    def paths(self) -> frozenset[str]:
        raise NotImplementedError

    @property
    @abstractmethod
    def canonical_path(self) -> str:
        raise NotImplementedError

    @property
    def model_paths(self) -> frozenset[str]:
        return frozenset()

    @abstractmethod
    def decode_request(self, document: dict[str, Any]) -> ScenarioRequest:
        raise NotImplementedError

    @abstractmethod
    def encode_response(
        self,
        request: ScenarioRequest,
        emission: ScenarioEmission,
        default_model: str,
    ) -> ProtocolResponse:
        raise NotImplementedError

    @abstractmethod
    def encode_error(
        self, status: int, code: str, message: str
    ) -> ProtocolResponse:
        raise NotImplementedError
