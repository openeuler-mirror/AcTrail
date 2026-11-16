"""Receive traffic at consecutive ports inside and outside an inclusive interval."""
import selectors
import socket
import threading
from ..continued.peers import NetworkPeers


class RangePeers(NetworkPeers):
    roles = ("control-low", "protected-low", "protected-middle", "protected-high", "control-middle", "control-high")

    def start(self):
        for version, family, host in ((4, socket.AF_INET, "127.0.0.2"), (6, socket.AF_INET6, "::1")):
            sockets = self.bind_interval(family, host)
            for protocol, kind in (("tcp", socket.SOCK_STREAM), ("udp", socket.SOCK_DGRAM)):
                for index, role in enumerate(self.roles):
                    prefix, position = role.split("-")
                    name = f"{prefix}-{protocol}{version}-{position}"
                    peer = sockets[protocol, index]
                    peer.setblocking(False)
                    if protocol == "tcp":
                        peer.listen()
                    self.selector.register(peer, selectors.EVENT_READ, (protocol, name, bytearray()))
                    self.endpoints[name] = {"host": host, "port": peer.getsockname()[1], "version": version,
                                            "protocol": protocol, "fastopen": False}
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()
        return self.endpoints

    def bind_interval(self, family, host):
        # Reserve real sockets together. An occupied neighboring port retries the allocation.
        for _ in range(32):
            sockets = {}
            try:
                first = socket.socket(family, socket.SOCK_STREAM)
                sockets["tcp", 0] = first
                first.bind((host, 0))
                base = first.getsockname()[1]
                if base + len(self.roles) > 65536:
                    raise OSError("ephemeral port too close to maximum")
                for protocol, kind in (("tcp", socket.SOCK_STREAM), ("udp", socket.SOCK_DGRAM)):
                    for index in range(len(self.roles)):
                        if (protocol, index) in sockets:
                            continue
                        peer = socket.socket(family, kind)
                        sockets[protocol, index] = peer
                        peer.bind((host, base + index))
                return sockets
            except OSError:
                for peer in sockets.values():
                    peer.close()
        raise RuntimeError("cannot reserve consecutive TCP/UDP witness ports")
