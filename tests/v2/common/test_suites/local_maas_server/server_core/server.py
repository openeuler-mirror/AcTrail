from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Mapping

from protocol import (
    AnthropicMessagesAdapter,
    OpenAIChatAdapter,
    ProtocolRegistry,
)
from record import (
    LocalMaaSRecordApplication,
    RecordSessionManager,
    RecordStore,
)
from scenario import ScenarioRuntime
from scenario.scenario_generator import ScenarioLoader
from scenario.tool_alias import ToolAliasConverterFactory
from schedule import ScheduleController
from transport import (
    LocalMaaSTransportApplication,
    TransportUpstreamResolver,
)
from utils import RequestLogger, StartupLogger

from .application import LocalMaaSApplication
from .api_endpoints import RestApi
from .connection.factory import ConnectionFactory
from .connection.interface import ConnectionDescription
from .connection.manager import ConnectionManager
from .help import HelpMessage, HelpSection

if TYPE_CHECKING:
    from config import LocalMaaSConfig


@dataclass(frozen=True, slots=True)
class LocalMaaSStatus:
    scenario: str
    description: str
    generator: str
    infinite: bool
    connections: Mapping[str, ConnectionDescription]
    warnings: tuple[str, ...]
    upstream: str | None = None
    upstream_model: str | None = None
    recordings_dir: str | None = None


class LocalMaaSServer:
    """Common server core: protocols, listeners, lifecycle, and startup log."""

    def __init__(self, config: LocalMaaSConfig):
        self._config = config
        self._application: object | None = None
        self._connections: ConnectionManager | None = None
        self._status: LocalMaaSStatus | None = None

    @property
    def status(self) -> LocalMaaSStatus | None:
        return self._status

    def start(self, *, silent: bool = False) -> LocalMaaSStatus:
        if self._connections is not None:
            raise RuntimeError("Local MaaS server is already running")

        connections: ConnectionManager | None = None
        try:
            protocols = ProtocolRegistry(
                (OpenAIChatAdapter(), AnthropicMessagesAdapter())
            )
            (
                application,
                scenario_id,
                description,
                generator_kind,
                infinite,
                upstream,
                upstream_model,
                recordings_dir,
            ) = self._application_and_status(protocols)
            rest_apis = self._register_endpoints(application)
            creation = ConnectionFactory().create(
                self._config.server,
                application,
                rest_apis,
            )
            connections = ConnectionManager(creation.servers)
            connections.start()
            status = LocalMaaSStatus(
                scenario=scenario_id,
                description=description,
                generator=generator_kind,
                infinite=infinite,
                connections=connections.describe(),
                warnings=creation.warnings,
                upstream=upstream,
                upstream_model=upstream_model,
                recordings_dir=recordings_dir,
            )
            self._connections = connections
            self._application = application
            self._status = status
            help_message = self.help_message(rest_apis)
            application.set_help_message(help_message)
            if not silent:
                StartupLogger().ready(
                    help=help_message,
                    connections=status.connections,
                    warnings=status.warnings,
                )
            return status
        except Exception:
            if connections is not None:
                connections.close()
            self._application = None
            self._connections = None
            self._status = None
            raise

    def _application_and_status(
        self,
        protocols: ProtocolRegistry,
    ) -> tuple[
        object, str, str, str, bool, str | None, str | None, str | None
    ]:
        raise NotImplementedError

    def _register_endpoints(
        self,
        application: object,
    ) -> tuple[RestApi, ...]:
        return application.rest_apis()

    def help_message(self, rest_apis: tuple[RestApi, ...]) -> HelpMessage:
        return HelpMessage(
            title=self._help_title(),
            sections=self._help_sections(rest_apis),
        )

    def _help_title(self) -> str:
        return "local MaaS replay server"

    def _help_sections(
        self,
        rest_apis: tuple[RestApi, ...],
    ) -> tuple[HelpSection, ...]:
        status = self._status
        if status is None:
            return (self._example_section(),)
        sections = [
            HelpSection(
                "Scenario",
                (
                    f"template:    {status.scenario}",
                    f"description: {status.description}",
                    f"generator:   {status.generator} "
                    f"({'infinite' if status.infinite else 'finite'})",
                ),
            )
        ]
        if status.upstream is not None:
            sections.append(
                HelpSection(
                    "Upstream",
                    (
                        f"base url:    {status.upstream}",
                        (
                            f"model:       {status.upstream_model}"
                            if status.upstream_model
                            else "model:       (use request model)"
                        ),
                    ),
                )
            )
        if status.recordings_dir is not None:
            sections.append(
                HelpSection(
                    "Recording",
                    (f"recordings dir: {status.recordings_dir}",),
                )
            )
        sections.append(
            HelpSection(
                "Endpoints",
                tuple(
                    f"{route.method} {route.path}"
                    for route in rest_apis
                ),
            )
        )
        sections.append(self._example_section())
        return tuple(sections)

    def _example_section(self) -> HelpSection:
        return HelpSection(
            "Example",
            (
                "curl -X POST {origin}/v1/chat/completions \\",
                "  -H 'Content-Type: application/json' \\",
                "  -d '{\"model\":\"local-maas-test\","
                "\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}],"
                "\"stream\":true}'",
            ),
        )

    def reset(self) -> None:
        application = self._application
        if application is None:
            raise RuntimeError("Local MaaS server is not running")
        if hasattr(application, "reset"):
            application.reset()

    def stop(self) -> None:
        connections = self._connections
        self._application = None
        self._connections = None
        self._status = None
        if connections is not None:
            connections.close()


class ScenarioReplayServer(LocalMaaSServer):
    def _application_and_status(
        self,
        protocols: ProtocolRegistry,
    ) -> tuple[
        object, str, str, str, bool, str | None, str | None, str | None
    ]:
        definition = ScenarioLoader(
            self._config.generator,
            protocols.names,
        ).load()
        application = LocalMaaSApplication(
            protocol_config=self._config.protocol,
            protocols=protocols,
            scenario=ScenarioRuntime(
                definition,
                ToolAliasConverterFactory().create(
                    self._config.tool_alias
                ),
            ),
            scheduler=ScheduleController(self._config.schedule),
            request_logger=RequestLogger(
                self._config.server.log_requests
            ),
            api_key=self._config.server.api_key,
        )
        return (
            application,
            definition.scenario_id,
            definition.description,
            definition.generator.kind,
            definition.generator.is_infinite,
            None,
            None,
            None,
        )


class TransportServer(LocalMaaSServer):
    def _application_and_status(
        self,
        protocols: ProtocolRegistry,
    ) -> tuple[
        object, str, str, str, bool, str | None, str | None, str | None
    ]:
        resolver = TransportUpstreamResolver(
            self._config.server.request_timeout_seconds
        )
        transport = resolver.resolve(self._config.transport)
        application = LocalMaaSTransportApplication(
            protocol_config=self._config.protocol,
            protocols=protocols,
            upstream=transport.upstream,
            request_logger=RequestLogger(
                self._config.server.log_requests
            ),
            request_timeout_seconds=(
                self._config.server.request_timeout_seconds
            ),
            api_key=self._config.server.api_key,
        )
        return (
            application,
            "transport",
            "transparent proxy: forward requests to an upstream MaaS",
            "transport",
            True,
            transport.upstream.base_url,
            transport.upstream.model,
            None,
        )

    def _help_title(self) -> str:
        return "local MaaS transport server"

    def _example_section(self) -> HelpSection:
        return HelpSection(
            "Example",
            (
                "curl -X POST {origin}/v1/chat/completions \\",
                "  -H 'Content-Type: application/json' \\",
                "  -d '{\"model\":\"local-maas-test\","
                "\"messages\":[{\"role\":\"user\",\"content\":\"hi\"}]}'",
            ),
        )


class ScenarioRecordServer(TransportServer):
    def _application_and_status(
        self,
        protocols: ProtocolRegistry,
    ) -> tuple[
        object, str, str, str, bool, str | None, str | None, str | None
    ]:
        recordings_dir = self._config.record.recordings_dir
        recordings_dir.mkdir(parents=True, exist_ok=True)
        store = RecordStore(
            recordings_dir,
            templates_dir=self._config.generator.templates_dir,
            supported_protocols=protocols.names,
            max_template_bytes=(
                self._config.generator.max_template_bytes
            ),
            max_depth=self._config.generator.max_depth,
            max_nodes=self._config.generator.max_nodes,
            random_seed=self._config.generator.random_seed,
        )
        application = LocalMaaSRecordApplication(
            protocol_config=self._config.protocol,
            protocols=protocols,
            sessions=RecordSessionManager(store),
            tool_aliases=self._config.tool_alias,
            request_logger=RequestLogger(
                self._config.server.log_requests
            ),
            request_timeout_seconds=(
                self._config.server.request_timeout_seconds
            ),
        )
        return (
            application,
            "record",
            "recording mode: forward requests to a real upstream MaaS",
            "record",
            True,
            None,
            None,
            str(recordings_dir),
        )

    def _register_endpoints(
        self,
        application: object,
    ) -> tuple[RestApi, ...]:
        routes = list(super()._register_endpoints(application))
        routes.extend(
            [
                RestApi("Recording", "POST", "/record/sessions"),
                RestApi("Recording", "GET", "/record/sessions"),
                RestApi(
                    "Recording",
                    "POST",
                    "/record/sessions/{session_id}/finalize",
                ),
            ]
        )
        return tuple(routes)

    def _help_title(self) -> str:
        return "local MaaS record server"

    def _help_sections(
        self,
        rest_apis: tuple[RestApi, ...],
    ) -> tuple[HelpSection, ...]:
        sections = super()._help_sections(rest_apis)
        return sections + (
            HelpSection(
                "Record API",
                (
                    "curl -X POST {origin}/record/sessions \\",
                    "  -H 'Content-Type: application/json' \\",
                    "  -d '{\"tools\":[\"read_file\",\"glob\",\"grep\"]}'",
                    "# 201 -> {\"session_id\":\"...\",\"api_key\":\"...\","
                    "\"state\":\"open\",\"response_count\":0,"
                    "\"cache_file\":\"...\"}",
                    "curl -X POST "
                    "{origin}/record/sessions/<session_id>/finalize \\",
                    "  -H 'Content-Type: application/json' \\",
                    "  -d '{\"scenario_id\":\"recorded-demo\"}'",
                    "# 200 -> {\"session_id\":\"...\","
                    "\"scenario_id\":\"recorded/...\","
                    "\"scenario_file\":\"...\",\"responses\":N}",
                ),
            ),
        )

    def _example_section(self) -> HelpSection:
        return HelpSection(
            "Example",
            (
                "1. create a recording session (see Record API)",
                "2. point your agent at {origin} with the session api key",
                "3. finalize to produce a replayable recorded scenario",
            ),
        )
