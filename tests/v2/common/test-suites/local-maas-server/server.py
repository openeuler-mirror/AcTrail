#!/usr/bin/env python3
from __future__ import annotations

import signal
import sys
from pathlib import Path
from types import FrameType

sys.path.insert(0, str(Path(__file__).resolve().parents[5]))

from config import ConfigurationError, LocalMaaSConfigParser
from protocol import (
    AnthropicMessagesAdapter,
    OpenAIChatAdapter,
    ProtocolRegistry,
)
from scenario import ScenarioConfigurationError, ScenarioRuntime
from scenario.scenario_generator import ScenarioLoader
from schedule import ScheduleController
from server_core import LocalMaaSApplication
from server_core.connection.factory import ConnectionFactory
from server_core.connection.interface import ConnectionStartupError
from server_core.connection.manager import ConnectionManager
from utils import RequestLogger, StartupLogger


class LocalMaaSServerCommand:
    def run(self) -> int:
        try:
            config = LocalMaaSConfigParser().parse()
            protocols = ProtocolRegistry(
                (OpenAIChatAdapter(), AnthropicMessagesAdapter())
            )
            definition = ScenarioLoader(
                config.generator,
                protocols.names,
            ).load()
            scenario = ScenarioRuntime(definition)
            application = LocalMaaSApplication(
                protocol_config=config.protocol,
                protocols=protocols,
                scenario=scenario,
                scheduler=ScheduleController(config.schedule),
                request_logger=RequestLogger(config.server.log_requests),
            )
            creation = ConnectionFactory().create(config.server, application)
            connections = ConnectionManager(creation.servers)
            connections.start()
        except (
            ConfigurationError,
            ConnectionStartupError,
            OSError,
            ScenarioConfigurationError,
            ValueError,
        ) as error:
            print(
                f"local_maas_startup_error={error}",
                file=sys.stderr,
                flush=True,
            )
            return 2

        StartupLogger().ready(
            connections=connections.describe(),
            scenario=definition.scenario_id,
            description=definition.description,
            generator=definition.generator.kind,
            infinite=definition.generator.is_infinite,
            warnings=creation.warnings,
        )
        signal.signal(signal.SIGTERM, self._terminate)
        try:
            connections.wait()
        except KeyboardInterrupt:
            pass
        finally:
            connections.close()
        return 0

    @staticmethod
    def _terminate(
        _signal_number: int,
        _frame: FrameType | None,
    ) -> None:
        raise KeyboardInterrupt


if __name__ == "__main__":
    raise SystemExit(LocalMaaSServerCommand().run())
