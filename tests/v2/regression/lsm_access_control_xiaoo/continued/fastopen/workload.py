"""Send real TCP Fast Open data and require SYN-data acknowledgement."""

import errno
import socket
import struct
import time


class FastOpenOperation:
    TCP_FASTOPEN_NO_COOKIE = 34
    TCP_CLOSE = 7
    TCP_ESTABLISHED = 1
    TCPI_OPT_SYN_DATA = 32

    def __init__(self, timeout, poll):
        self.timeout = timeout
        self.poll = poll

    def send(self, endpoint, payload):
        family = socket.AF_INET6 if endpoint["version"] == 6 else socket.AF_INET
        with socket.socket(family, socket.SOCK_STREAM) as peer:
            # Python settimeout makes the socket nonblocking; use SO_SNDTIMEO
            # to retain the blocking Fast Open path with a bounded wait.
            seconds = int(self.timeout)
            microseconds = int((self.timeout - seconds) * 1_000_000)
            peer.setsockopt(socket.SOL_SOCKET, socket.SO_SNDTIMEO,
                            struct.pack("ll", seconds, microseconds))
            peer.setsockopt(socket.IPPROTO_TCP, self.TCP_FASTOPEN_NO_COOKIE, 1)
            if self.info(peer)[0] != self.TCP_CLOSE:
                raise AssertionError("Fast Open must start from an unconnected socket")
            try:
                written = peer.sendto(payload, socket.MSG_FASTOPEN, (endpoint["host"], endpoint["port"]))
            except OSError as error:
                if error.errno in (errno.EACCES, errno.EPERM) and self.info(peer)[0] != self.TCP_CLOSE:
                    raise AssertionError("denied Fast Open changed the socket connection state") from error
                raise
            deadline = time.monotonic() + self.timeout
            while time.monotonic() < deadline:
                info = self.info(peer)
                if info[0] == self.TCP_ESTABLISHED:
                    if not info[5] & self.TCPI_OPT_SYN_DATA:
                        raise AssertionError("Fast Open fell back: SYN data was not acknowledged")
                    return written
                time.sleep(self.poll)
            raise TimeoutError("Fast Open handshake did not finish")

    @staticmethod
    def info(peer):
        return peer.getsockopt(socket.IPPROTO_TCP, socket.TCP_INFO, 104)
