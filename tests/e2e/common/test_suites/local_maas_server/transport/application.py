from __future__ import annotations

import json
from typing import Any, Iterator, Mapping

from protocol import (
    ProtocolConfig,
    ProtocolFrame,
    ProtocolRegistry,
    ProtocolResponse,
)
from protocol.interface import ProtocolAdapter
from server_core.api_endpoints import ApiEndpoints, ProtocolEndpoint, RestApi
from server_core.application import (
    ApplicationResponse,
    RequestCompletion,
    extract_api_keys,
)
from server_core.errors import ConnectionRequestError
from server_core.help import HelpMessageMixin
from utils import RequestLogger

from .upstream import (
    DirectUpstreamResponse,
    OpenAIUpstreamClient,
    StreamingUpstreamResponse,
    UpstreamConfig,
)


class LocalMaaSTransportApplication(HelpMessageMixin):
    """Transparent LLM proxy application.

    Subclasses override the capture hooks to observe forwarded exchanges
    (recording mode) without duplicating the forwarding path.
    """

    def __init__(
        self,
        *,
        protocol_config: ProtocolConfig,
        protocols: ProtocolRegistry,
        upstream: UpstreamConfig | None,
        request_logger: RequestLogger,
        request_timeout_seconds: float,
        api_key: str | None,
    ):
        self._protocol_config = protocol_config
        self._endpoints = ApiEndpoints(protocols)
        self._upstream = upstream
        self._request_logger = request_logger
        self._api_key = api_key
        self._request_timeout = request_timeout_seconds
        self._client = OpenAIUpstreamClient(request_timeout_seconds)

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

    def reset(self) -> None:
        return

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
            value.encode("utf-8") == expected_bytes for value in supplied
        ):
            raise ConnectionRequestError(
                401,
                "invalid_api_key",
                "request did not provide the configured local API key",
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
                    404,
                    "route_not_found",
                    f"no MaaS endpoint at {path}",
                ),
                completion=None,
            )
        if adapter.name != "openai":
            return ApplicationResponse(
                response=self.generic_error(
                    501,
                    "unsupported_upstream_protocol",
                    "transport mode only supports an OpenAI-compatible "
                    "upstream MaaS",
                ),
                completion=None,
            )
        context = self._context_for(headers)
        if context is None:
            return ApplicationResponse(
                response=self.generic_error(
                    401,
                    "invalid_api_key",
                    "request does not map to an upstream",
                ),
                completion=None,
            )
        stream = document.get("stream", False)
        if not isinstance(stream, bool):
            return ApplicationResponse(
                response=adapter.encode_error(
                    400, "invalid_stream", "stream must be a boolean"
                ),
                completion=None,
            )
        try:
            outbound = self._transform_request(context, document)
            upstream = self._client.forward(
                self._upstream_for(context),
                adapter.canonical_path,
                outbound,
                stream=stream,
            )
        except (OSError, ValueError) as error:
            return ApplicationResponse(
                response=self.generic_error(
                    502,
                    "upstream_unavailable",
                    f"upstream MaaS request failed: {error}",
                ),
                completion=None,
            )
        if isinstance(upstream, DirectUpstreamResponse):
            return self._handle_direct(
                context, adapter, stream, upstream
            )
        return self._handle_stream(
            context, adapter, stream, upstream
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

    def health(self, origin: str) -> dict[str, Any]:
        return {
            "status": "ok",
            "mode": "transport",
            "scenario": "transport",
            "description": (
                "transparent proxy: forward requests to an upstream MaaS"
            ),
            "upstream": self._upstream.base_url
            if self._upstream is not None
            else None,
            "model": self._upstream.model
            if self._upstream is not None
            else None,
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

    def error_for_path(
        self,
        path: str,
        status: int,
        code: str,
        message: str,
    ) -> ProtocolResponse:
        adapter = self._endpoints.resolve("POST", path)
        if adapter is None:
            return self.generic_error(status, code, message)
        return adapter.encode_error(status, code, message)

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

    def _context_for(
        self,
        headers: Mapping[str, str] | None,
    ) -> object | None:
        return self._upstream

    def _upstream_for(self, context: object) -> UpstreamConfig:
        if not isinstance(context, UpstreamConfig):
            raise RuntimeError("transport context must be an upstream")
        return context

    def _transform_request(
        self,
        context: object,
        document: dict[str, Any],
    ) -> dict[str, Any]:
        outbound = dict(document)
        upstream = self._upstream_for(context)
        if upstream.model:
            outbound["model"] = upstream.model
        return outbound

    def _capture_direct(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        body: bytes,
    ) -> None:
        return

    def _stream_parser(
        self,
        protocol: str,
    ) -> object | None:
        return None

    def _capture_stream(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        parser: object,
    ) -> None:
        return

    def _completion(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        status: int,
    ) -> RequestCompletion:
        return RequestCompletion(
            protocol=adapter.name,
            template_path="transport",
            stream=stream,
            status=status,
        )

    def _handle_direct(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        upstream: DirectUpstreamResponse,
    ) -> ApplicationResponse:
        if not upstream.is_error:
            self._capture_direct(
                context, adapter, stream, upstream.body
            )
        return ApplicationResponse(
            response=ProtocolResponse(
                status=upstream.status,
                media_type=upstream.media_type,
                body=upstream.body,
            ),
            completion=self._completion(
                context, adapter, stream, upstream.status
            ),
        )

    def _handle_stream(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        upstream: StreamingUpstreamResponse,
    ) -> ApplicationResponse:
        if upstream.is_error:
            return ApplicationResponse(
                response=ProtocolResponse(
                    status=upstream.status,
                    media_type=upstream.media_type,
                    body=upstream.body or b"",
                ),
                completion=self._completion(
                    context, adapter, stream, upstream.status
                ),
            )
        upstream_stream = upstream.stream
        parser = self._stream_parser(adapter.name)

        def frames() -> Iterator[ProtocolFrame]:
            try:
                for line in upstream_stream.lines():
                    if parser is not None:
                        parser.feed_line(line)
                    yield ProtocolFrame(payload=line)
                if parser is not None:
                    self._capture_stream(
                        context, adapter, stream, parser
                    )
            finally:
                upstream_stream.close()

        return ApplicationResponse(
            response=ProtocolResponse(
                status=upstream.status,
                media_type=upstream.media_type,
                frames=frames(),
            ),
            completion=self._completion(
                context, adapter, stream, upstream.status
            ),
        )
