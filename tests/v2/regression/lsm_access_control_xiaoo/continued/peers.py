"""Record actual TCP and UDP payloads received during live policy changes."""

import selectors
import socket
import threading


class NetworkPeers:
    def __init__(self, timeout, poll, *, fastopen=False):
        self.timeout = timeout
        self.poll = poll
        self.selector = selectors.DefaultSelector()
        self.stopped = threading.Event()
        self.thread = None
        self.endpoints = {}
        self.received = set()
        self.failure = None
        self.fastopen = fastopen

    def start(self):
        for version, family, host in ((4, socket.AF_INET, "127.0.0.2"),
                                      (6, socket.AF_INET6, "::1")):
            for protocol, kind in (("udp", socket.SOCK_DGRAM), ("tcp", socket.SOCK_STREAM)):
                for role in ("protected", "control"):
                    name = f"{role}-{protocol}{version}"
                    peer = socket.socket(family, kind)
                    try:
                        peer.bind((host, 0))
                        peer.setblocking(False)
                        if protocol == "tcp":
                            if self.fastopen:
                                peer.setsockopt(socket.IPPROTO_TCP, socket.TCP_FASTOPEN, socket.SOMAXCONN)
                                peer.setsockopt(socket.IPPROTO_TCP, 34, 1)  # TCP_FASTOPEN_NO_COOKIE
                            peer.listen()
                        self.selector.register(peer, selectors.EVENT_READ, (protocol, name, bytearray()))
                    except BaseException:
                        peer.close()
                        raise
                    port = peer.getsockname()[1]
                    self.endpoints[name] = {"host": host, "port": port, "version": version,
                                            "protocol": protocol, "fastopen": self.fastopen}
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()
        return self.endpoints

    def serve(self):
        try:
            while not self.stopped.is_set():
                for key, _ in self.selector.select(self.poll):
                    kind, name, pending = key.data
                    peer = key.fileobj
                    if kind == "tcp":
                        connection, _ = peer.accept()
                        connection.setblocking(False)
                        self.selector.register(connection, selectors.EVENT_READ,
                                               ("stream", name, bytearray()))
                    else:
                        data = peer.recv(4096)
                        if not data and kind == "stream":
                            self.selector.unregister(peer)
                            peer.close()
                            continue
                        pending.extend(data)
                        while b"\n" in pending:
                            line, _, remainder = pending.partition(b"\n")
                            self.received.add((name, line.decode()))
                            pending[:] = remainder
        except BaseException as error:
            self.failure = str(error)

    def stop(self):
        self.stopped.set()
        if self.thread is not None:
            self.thread.join(self.timeout)
            if self.thread.is_alive():
                raise RuntimeError("network witness did not stop")
        for key in list(self.selector.get_map().values()):
            self.selector.unregister(key.fileobj)
            key.fileobj.close()
        self.selector.close()
        if self.failure:
            raise AssertionError(f"network witness failed: {self.failure}")
