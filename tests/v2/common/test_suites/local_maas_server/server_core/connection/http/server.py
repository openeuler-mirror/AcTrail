from __future__ import annotations

from http.server import ThreadingHTTPServer

from server_core.application import LocalMaaSApplication
from server_core.api_endpoints import RestApi
from server_core.config import ServerCoreConfig
from server_core.connection.interface import (
    ConnectionDescription,
    ConnectionServer,
)

from .config import HTTPConfig
from .handler import HTTPRequestHandler

from tests.v2.common.test_suites.local_maas_server.constants import DEFAULTS


class HTTPConnectionServer(ThreadingHTTPServer, ConnectionServer):
    allow_reuse_address = DEFAULTS.allow_reuse_address
    daemon_threads = DEFAULTS.daemon_threads

    def __init__(
        self,
        listener_config: HTTPConfig,
        server_config: ServerCoreConfig,
        application: LocalMaaSApplication,
        rest_apis: tuple[RestApi, ...] | None = None,
    ):
        self._server_config = server_config
        self._application = application
        self._rest_apis = rest_apis
        ThreadingHTTPServer.__init__(
            self,
            (listener_config.bind_host, listener_config.bind_port),
            HTTPRequestHandler,
        )
        host, port = self.server_address[:2]
        display_host = f"[{host}]" if ":" in host else host
        origin = f"{self.scheme}://{display_host}:{port}"
        self._description = ConnectionDescription(
            service=self.scheme.upper(),
            scheme=self.scheme,
            host=host,
            port=port,
            origin=origin,
            endpoints=application.protocol_endpoints(origin),
            rest_apis=(
                application.rest_apis()
                if self._rest_apis is None
                else self._rest_apis
            ),
        )

    @property
    def config(self) -> ServerCoreConfig:
        return self._server_config

    @property
    def application(self) -> LocalMaaSApplication:
        return self._application

    @property
    def scheme(self) -> str:
        return "http"

    @property
    def origin(self) -> str:
        return self._description.origin

    @property
    def description(self) -> ConnectionDescription:
        return self._description

    def serve_forever(self) -> None:
        ThreadingHTTPServer.serve_forever(self)

    def shutdown(self) -> None:
        ThreadingHTTPServer.shutdown(self)

    def close(self) -> None:
        self.server_close()
