from __future__ import annotations

import json
from pathlib import Path
from typing import Any, Mapping

from protocol import ProtocolConfig, ProtocolRegistry, ProtocolResponse
from protocol.interface import ProtocolAdapter
from scenario.tool_alias import ToolAliasConfig
from server_core.api_endpoints import ApiEndpoints
from server_core.application import (
    RequestCompletion,
    extract_api_keys,
)
from server_core.errors import ConnectionRequestError
from transport.application import LocalMaaSTransportApplication
from transport.pruner import ToolPruner
from transport.upstream import UpstreamConfig
from transport.upstream_resolver import (
    TransportUpstreamResolver,
    UpstreamResolutionError,
)
from utils import RequestLogger
from utils.json import StrictJsonDecoder, StrictJsonError

from .parser import RecordedResponse, ResponseParser
from .session import RecordSession, RecordSessionError, RecordSessionManager
from .store import RecordFinalizeError


class LocalMaaSRecordApplication(LocalMaaSTransportApplication):
    """Recording application: forward like transport mode and capture every
    complete upstream response into the session cache."""

    def __init__(
        self,
        *,
        protocol_config: ProtocolConfig,
        protocols: ProtocolRegistry,
        sessions: RecordSessionManager,
        tool_aliases: ToolAliasConfig,
        request_logger: RequestLogger,
        request_timeout_seconds: float,
    ):
        super().__init__(
            protocol_config=protocol_config,
            protocols=protocols,
            upstream=None,
            request_logger=request_logger,
            request_timeout_seconds=request_timeout_seconds,
            api_key=None,
        )
        self._sessions = sessions
        self._parser = ResponseParser(tool_aliases)
        self._endpoints = ApiEndpoints(protocols)

    def supports_management(self, method: str, path: str) -> bool:
        if method == "GET":
            return path == "/record/sessions"
        if method == "POST":
            return path == "/record/sessions" or (
                self._finalize_session_id(path) is not None
            )
        return False

    def handle_management(
        self,
        method: str,
        path: str,
        document: dict[str, Any] | None,
    ) -> ProtocolResponse:
        if method == "GET" and path == "/record/sessions":
            return self._json_response(200, self._sessions_payload())
        if method == "POST" and path == "/record/sessions":
            return self._create_session(document)
        session_id = self._finalize_session_id(path)
        if method == "POST" and session_id is not None:
            return self._finalize_session(session_id, document)
        return self.generic_error(
            404,
            "route_not_found",
            f"no recording endpoint at {path}",
        )

    def verify_request(self, headers: Mapping[str, str]) -> None:
        supplied = extract_api_keys(headers)
        if not supplied:
            raise ConnectionRequestError(
                401,
                "missing_api_key",
                "request did not provide a recording session API key",
            )
        for value in supplied:
            if self._sessions.get_by_api_key(value) is not None:
                return
        raise ConnectionRequestError(
            401,
            "invalid_api_key",
            "request API key does not match any recording session",
        )

    def health(self, origin: str) -> dict[str, Any]:
        payload = super().health(origin)
        payload["mode"] = "record"
        payload["description"] = (
            "recording mode: forward requests to a real upstream MaaS"
        )
        payload["sessions"] = len(self._sessions.list_sessions())
        payload.pop("upstream", None)
        return payload

    def _context_for(
        self,
        headers: Mapping[str, str] | None,
    ) -> RecordSession | None:
        if headers is None:
            return None
        for value in extract_api_keys(headers):
            session = self._sessions.get_by_api_key(value)
            if session is not None:
                return session
        return None

    def _upstream_for(self, context: object) -> UpstreamConfig:
        if not isinstance(context, RecordSession):
            raise RuntimeError("recording context must be a session")
        return context.upstream

    def _transform_request(
        self,
        context: object,
        document: dict[str, Any],
    ) -> dict[str, Any]:
        if not isinstance(context, RecordSession):
            raise RuntimeError("recording context must be a session")
        outbound = ToolPruner(context.tools).prune(document)
        if context.upstream.model:
            outbound["model"] = context.upstream.model
        return outbound

    def _capture_direct(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        body: bytes,
    ) -> None:
        if not isinstance(context, RecordSession):
            return
        recorded = self._record_direct(adapter, body)
        if recorded is not None:
            self._append_record(context, recorded)

    def _stream_parser(
        self,
        protocol: str,
    ) -> object | None:
        return self._parser.stream_parser(protocol)

    def _capture_stream(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        parser: object,
    ) -> None:
        if not isinstance(context, RecordSession):
            return
        recorded = parser.result()
        if recorded is not None:
            self._append_record(context, recorded)

    def _completion(
        self,
        context: object,
        adapter: ProtocolAdapter,
        stream: bool,
        status: int,
    ) -> RequestCompletion:
        session_id = (
            context.session_id
            if isinstance(context, RecordSession)
            else "record"
        )
        return RequestCompletion(
            protocol=adapter.name,
            template_path=session_id,
            stream=stream,
            status=status,
        )

    def _record_direct(
        self,
        adapter: ProtocolAdapter,
        body: bytes,
    ) -> RecordedResponse | None:
        try:
            document = StrictJsonDecoder().decode_utf8(body)
        except StrictJsonError:
            return None
        if not isinstance(document, dict):
            return None
        return self._parser.parse_direct(adapter.name, document)

    def _append_record(
        self,
        session: RecordSession,
        recorded: RecordedResponse,
    ) -> None:
        try:
            self._sessions.append(session, recorded)
        except RecordSessionError:
            return

    def _create_session(
        self,
        document: dict[str, Any] | None,
    ) -> ProtocolResponse:
        try:
            tools = self._require_tools(document)
            upstream = self._require_upstream(document)
            if upstream is None:
                resolved = TransportUpstreamResolver(
                    self._request_timeout
                ).resolve(
                    None,
                    context="recording session",
                )
                upstream = resolved.upstream
            session = self._sessions.create(tools, upstream)
        except (
            TypeError,
            UpstreamResolutionError,
            ValueError,
        ) as error:
            return self.generic_error(
                400, "invalid_session", str(error)
            )
        return self._json_response(201, self._session_payload(session))

    def _finalize_session(
        self,
        session_id: str,
        document: dict[str, Any] | None,
    ) -> ProtocolResponse:
        try:
            scenario_id = self._scenario_id(document, session_id)
            result = self._sessions.finalize(session_id, scenario_id)
        except RecordSessionError as error:
            status = 404 if error.code == "session_not_found" else 409
            return self.generic_error(
                status, error.code, str(error)
            )
        except RecordFinalizeError as error:
            status = 400 if error.code == "invalid_cache" else 409
            return self.generic_error(
                status, error.code, str(error)
            )
        except ValueError as error:
            return self.generic_error(
                400, "invalid_scenario_id", str(error)
            )
        return self._json_response(
            200,
            {
                "session_id": session_id,
                "scenario_id": result.scenario_id,
                "scenario_file": str(result.scenario_file),
                "responses": result.responses,
            },
        )

    def _sessions_payload(self) -> dict[str, Any]:
        return {
            "sessions": [
                self._session_payload(session)
                for session in self._sessions.list_sessions()
            ]
        }

    @staticmethod
    def _session_payload(session: RecordSession) -> dict[str, Any]:
        return {
            "session_id": session.session_id,
            "api_key": session.api_key,
            "state": session.state,
            "response_count": session.response_count,
            "cache_file": str(session.cache_path),
            "tools": sorted(session.tools),
            "upstream": {
                "base_url": session.upstream.base_url,
                "model": session.upstream.model,
            },
        }

    @staticmethod
    def _require_tools(
        document: dict[str, Any] | None,
    ) -> tuple[str, ...]:
        if not isinstance(document, dict):
            raise ValueError("request body must be a JSON object")
        raw_tools = document.get("tools")
        if not isinstance(raw_tools, list) or not raw_tools:
            raise ValueError("tools must be a non-empty array")
        tools: list[str] = []
        for index, tool in enumerate(raw_tools):
            if not isinstance(tool, str) or not tool:
                raise ValueError(
                    f"tools[{index}] must be a non-empty string"
                )
            tools.append(tool)
        return tuple(tools)

    @staticmethod
    def _require_upstream(
        document: dict[str, Any] | None,
    ) -> UpstreamConfig | None:
        if not isinstance(document, dict):
            raise ValueError("request body must be a JSON object")
        raw_upstream = document.get("upstream")
        if raw_upstream is None:
            return None
        if not isinstance(raw_upstream, dict):
            raise ValueError("upstream must be an object")
        base_url = raw_upstream.get("base_url")
        api_key = raw_upstream.get("api_key")
        if not isinstance(base_url, str) or not base_url:
            raise ValueError("upstream.base_url must be a non-empty string")
        if not isinstance(api_key, str) or not api_key:
            raise ValueError("upstream.api_key must be a non-empty string")
        model = raw_upstream.get("model")
        if model is not None and (
            not isinstance(model, str) or not model
        ):
            raise ValueError("upstream.model must be a non-empty string")
        return UpstreamConfig(
            base_url=base_url,
            api_key=api_key,
            model=model,
        )

    @staticmethod
    def _scenario_id(
        document: dict[str, Any] | None,
        session_id: str,
    ) -> str:
        scenario_id = f"recorded-{session_id}"
        if isinstance(document, dict):
            requested = document.get("scenario_id")
            if requested is not None:
                if not isinstance(requested, str) or not requested:
                    raise ValueError(
                        "scenario_id must be a non-empty string"
                    )
                scenario_id = requested
        path = Path(scenario_id)
        if path.is_absolute() or any(part == ".." for part in path.parts):
            raise ValueError(
                "scenario_id must be a relative path without .."
            )
        return scenario_id

    @staticmethod
    def _finalize_session_id(path: str) -> str | None:
        parts = [part for part in path.split("/") if part]
        if (
            len(parts) == 4
            and parts[0] == "record"
            and parts[1] == "sessions"
            and parts[3] == "finalize"
        ):
            return parts[2]
        return None

    @staticmethod
    def _json_response(
        status: int, payload: dict[str, Any]
    ) -> ProtocolResponse:
        body = json.dumps(
            payload,
            ensure_ascii=False,
            separators=(",", ":"),
        ).encode("utf-8")
        return ProtocolResponse(
            status=status,
            media_type="application/json; charset=utf-8",
            body=body,
        )
