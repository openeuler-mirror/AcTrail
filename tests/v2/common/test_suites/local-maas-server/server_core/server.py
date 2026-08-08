from __future__ import annotations

from dataclasses import dataclass
from typing import TYPE_CHECKING, Mapping

from protocol import (
    AnthropicMessagesAdapter,
    OpenAIChatAdapter,
    ProtocolRegistry,
)
from scenario import ScenarioRuntime
from scenario.scenario_generator import ScenarioLoader
from scenario.tool_alias import ToolAliasConverterFactory
from schedule import ScheduleController
from utils import RequestLogger, StartupLogger

from .application import LocalMaaSApplication
from .connection.factory import ConnectionFactory
from .connection.interface import ConnectionDescription
from .connection.manager import ConnectionManager

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


class LocalMaaSServer:
    def __init__(self, config: LocalMaaSConfig):
        self._config = config
        self._application: LocalMaaSApplication | None = None
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
            )
            creation = ConnectionFactory().create(
                self._config.server,
                application,
            )
            connections = ConnectionManager(creation.servers)
            connections.start()
            status = LocalMaaSStatus(
                scenario=definition.scenario_id,
                description=definition.description,
                generator=definition.generator.kind,
                infinite=definition.generator.is_infinite,
                connections=connections.describe(),
                warnings=creation.warnings,
            )
            self._connections = connections
            self._application = application
            self._status = status
            if not silent:
                StartupLogger().ready(
                    connections=status.connections,
                    scenario=status.scenario,
                    description=status.description,
                    generator=status.generator,
                    infinite=status.infinite,
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

    def reset(self) -> None:
        application = self._application
        if application is None:
            raise RuntimeError("Local MaaS server is not running")
        application.reset()

    def stop(self) -> None:
        connections = self._connections
        self._application = None
        self._connections = None
        self._status = None
        if connections is not None:
            connections.close()
