from __future__ import annotations

import ssl
from dataclasses import replace
from socket import socket
from typing import Any

from server_core.application import LocalMaaSApplication
from server_core.config import ServerCoreConfig

from ..http.config import HTTPConfig
from ..http.server import HTTPConnectionServer
from .certificate import EphemeralCertificate
from .config import HTTPSConfig


class HTTPSConnectionServer(HTTPConnectionServer):
    def __init__(
        self,
        listener_config: HTTPSConfig,
        server_config: ServerCoreConfig,
        application: LocalMaaSApplication,
    ):
        self._certificate = EphemeralCertificate(listener_config)
        self._ssl_context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        self._ssl_context.set_alpn_protocols(["http/1.1"])
        try:
            self._ssl_context.load_cert_chain(
                certfile=self._certificate.server_cert_file,
                keyfile=self._certificate.server_key_file,
            )
            super().__init__(
                HTTPConfig(
                    bind_host=listener_config.bind_host,
                    bind_port=listener_config.bind_port,
                ),
                server_config,
                application,
            )
        except Exception:
            self._certificate.close()
            raise
        self._description = replace(
            self._description,
            ca_cert_file=str(self._certificate.ca_cert_file),
        )
        self._closed = False

    @property
    def scheme(self) -> str:
        return "https"

    def get_request(self) -> tuple[socket, Any]:
        connection, address = super().get_request()
        try:
            secure_connection = self._ssl_context.wrap_socket(
                connection,
                server_side=True,
                do_handshake_on_connect=False,
            )
        except Exception:
            connection.close()
            raise
        return secure_connection, address

    def close(self) -> None:
        if self._closed:
            return
        self._closed = True
        try:
            super().close()
        finally:
            self._certificate.close()
