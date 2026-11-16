from __future__ import annotations

import hmac
import json
from dataclasses import dataclass, replace
from typing import Any, Mapping

from protocol import ProtocolConfig, ProtocolRegistry, ProtocolResponse
from protocol.interface import ProtocolRequestError
from scenario import ScenarioRuntime, ScenarioRuntimeError
from schedule import ScheduleController
from utils import RequestLogger

from .api_endpoints import ApiEndpoints, ProtocolEndpoint, RestApi
from .errors import ConnectionRequestError
from .help import HelpMessageMixin


def extract_api_keys(headers: Mapping[str, str]) -> list[str]:
    supplied: list[str] = []
    authorization = headers.get("Authorization")
    if authorization is not None:
        scheme, separator, value = authorization.partition(" ")
        if separator and scheme.lower() == "bearer" and value:
            supplied.append(value)
    anthropic_key = headers.get("X-Api-Key")
    if anthropic_key is not None:
        supplied.append(anthropic_key)
    return supplied


@dataclass(frozen=True, slots=True)
class RequestCompletion:
    protocol: str
    template_path: str
    stream: bool
    status: int


@dataclass(frozen=True, slots=True)
class ApplicationResponse:
    response: ProtocolResponse
    completion: RequestCompletion | None


class LocalMaaSApplication(HelpMessageMixin):
    _SCENARIO_ERROR_STATUS = {
        "scenario_exhausted": 409,
        "scenario_mismatch": 409,
    }

    def __init__(
        self,
        *,
        protocol_config: ProtocolConfig,
        protocols: ProtocolRegistry,
        scenario: ScenarioRuntime,
        scheduler: ScheduleController,
        request_logger: RequestLogger,
        api_key: str | None,
    ):
        self._protocol_config = protocol_config
        self._endpoints = ApiEndpoints(protocols)
        self._scenario = scenario
        self._scheduler = scheduler
        self._request_logger = request_logger
        self._api_key = api_key

    def verify_request(self, headers: Mapping[str, str]) -> None:
        expected = self._api_key
        if expected is None:
            return
        supplied = extract_api_keys(headers)
        if not supplied:
            raise ConnectionRequestError(
                401,
                "missing_api_key",
                "request did not provide the configured local API key",
            )
        expected_bytes = expected.encode("utf-8")
        if not any(
            hmac.compare_digest(value.encode("utf-8"), expected_bytes)
            for value in supplied
        ):
            raise ConnectionRequestError(
                401,
                "invalid_api_key",
                "request did not provide the configured local API key",
            )

    def reset(self) -> None:
        self._scenario.reset()

    def supports_path(self, path: str) -> bool:
        return self._endpoints.resolve("POST", path) is not None

    def supports_models_path(self, path: str) -> bool:
        return self._endpoints.supports_models(path)

    def supports_health_path(self, path: str) -> bool:
        return self._endpoints.supports_health(path)

    def supports_management(self, method: str, path: str) -> bool:
        return False

    def supports_reset_path(self, path: str) -> bool:
        return path == "/reset"

    def handle_management(
        self,
        method: str,
        path: str,
        document: dict[str, Any] | None,
    ) -> ProtocolResponse:
        return self.generic_error(
            404,
            "route_not_found",
            f"no recording endpoint at {path}",
        )

    def handle_post(
        self,
        path: str,
        document: dict[str, Any],
        headers: Mapping[str, str] | None = None,
    ) -> ApplicationResponse:
        adapter = self._endpoints.resolve("POST", path)
        if adapter is None:
            return ApplicationResponse(
                response=self.generic_error(
                    404, "route_not_found", f"no MaaS endpoint at {path}"
                ),
                completion=None,
            )
        try:
            request = adapter.decode_request(document)
            emission = self._scenario.reserve(request)
            response = adapter.encode_response(
                request,
                emission,
                self._protocol_config.default_model,
            )
            if response.frames is not None:
                response = replace(
                    response,
                    frames=self._scheduler.apply(response.frames),
                )
            return ApplicationResponse(
                response=response,
                completion=RequestCompletion(
                    protocol=adapter.name,
                    template_path=emission.template.source_path,
                    stream=request.stream,
                    status=response.status,
                ),
            )
        except ProtocolRequestError as error:
            return ApplicationResponse(
                response=adapter.encode_error(400, error.code, str(error)),
                completion=None,
            )
        except ScenarioRuntimeError as error:
            status = self._SCENARIO_ERROR_STATUS.get(error.code, 500)
            return ApplicationResponse(
                response=adapter.encode_error(status, error.code, str(error)),
                completion=None,
            )

    def record_completed(self, result: ApplicationResponse) -> None:
        completion = result.completion
        if completion is None:
            return
        self._request_logger.completed(
            protocol=completion.protocol,
            template_path=completion.template_path,
            stream=completion.stream,
            status=completion.status,
        )

    def error_for_path(
        self, path: str, status: int, code: str, message: str
    ) -> ProtocolResponse:
        adapter = self._endpoints.resolve("POST", path)
        if adapter is None:
            return self.generic_error(status, code, message)
        return adapter.encode_error(status, code, message)

    def health(self, origin: str) -> dict[str, Any]:
        definition = self._scenario.definition
        generator = definition.generator
        return {
            "status": "ok",
            "scenario": definition.scenario_id,
            "description": definition.description,
            "generator": {
                "type": generator.kind,
                "infinite": generator.is_infinite,
                "exhaustible": not generator.is_infinite,
            },
            "endpoints": self.endpoints(origin),
        }

    def endpoints(self, origin: str) -> dict[str, str]:
        return self._endpoints.describe(origin)

    def protocol_endpoints(
        self,
        origin: str,
    ) -> tuple[ProtocolEndpoint, ...]:
        return self._endpoints.describe_protocol_endpoints(origin)

    def rest_apis(self) -> tuple[RestApi, ...]:
        return self._endpoints.describe_rest_apis()

    def models(self) -> dict[str, Any]:
        return {
            "object": "list",
            "data": [
                {
                    "id": self._protocol_config.default_model,
                    "object": "model",
                    "created": 0,
                    "owned_by": "local-maas",
                }
            ],
        }

    @staticmethod
    def generic_error(
        status: int, code: str, message: str
    ) -> ProtocolResponse:
        body = json.dumps(
            {"error": {"code": code, "message": message}},
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        return ProtocolResponse(
            status=status,
            media_type="application/json; charset=utf-8",
            body=body,
        )
