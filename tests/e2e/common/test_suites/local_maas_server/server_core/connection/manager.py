from __future__ import annotations

from threading import Thread

from .interface import ConnectionDescription, ConnectionServer


class ConnectionManager:
    def __init__(
        self,
        servers: tuple[ConnectionServer, ...],
    ):
        if not servers:
            raise ValueError("at least one connection server is required")
        schemes = [server.scheme for server in servers]
        if len(set(schemes)) != len(schemes):
            raise ValueError("connection server schemes must be unique")
        self._servers = servers
        self._threads: list[Thread] = []

    def start(self) -> None:
        for server in self._servers:
            thread = Thread(
                target=server.serve_forever,
                name=f"local-maas-{server.scheme}",
                daemon=True,
            )
            thread.start()
            self._threads.append(thread)

    def describe(self) -> dict[str, ConnectionDescription]:
        return {
            server.scheme: server.description for server in self._servers
        }

    def wait(self) -> None:
        for thread in self._threads:
            thread.join()

    def close(self) -> None:
        for server, thread in zip(self._servers, self._threads):
            if thread.is_alive():
                server.shutdown()
        for thread in self._threads:
            thread.join()
        for server in reversed(self._servers):
            server.close()
        self._threads.clear()
