from __future__ import annotations

from dataclasses import dataclass

from server_core.application import LocalMaaSApplication
from server_core.api_endpoints import RestApi
from server_core.config import ServerCoreConfig

from .http.server import HTTPConnectionServer
from .https.server import HTTPSConnectionServer
from .interface import ConnectionServer


@dataclass(frozen=True, slots=True)
class ConnectionCreation:
    servers: tuple[ConnectionServer, ...]
    warnings: tuple[str, ...]


class ConnectionFactory:
    def create(
        self,
        config: ServerCoreConfig,
        application: LocalMaaSApplication,
        rest_apis: tuple[RestApi, ...] | None = None,
    ) -> ConnectionCreation:
        servers: list[ConnectionServer] = []
        warnings: list[str] = []
        try:
            servers.append(
                HTTPConnectionServer(
                    config.http, config, application, rest_apis
                )
            )
            if config.https is not None:
                try:
                    servers.append(
                        HTTPSConnectionServer(
                            config.https,
                            config,
                            application,
                            rest_apis,
                        )
                    )
                except Exception as error:
                    if not config.https.best_effort:
                        raise
                    warnings.append(f"HTTPS was not started: {error}")
        except Exception:
            for server in reversed(servers):
                server.close()
            raise
        return ConnectionCreation(
            servers=tuple(servers),
            warnings=tuple(warnings),
        )
