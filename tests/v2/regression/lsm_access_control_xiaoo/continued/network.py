"""Send through retained TCP/UDP sockets and explicit UDP destinations."""

import ctypes
import json
import os
import socket
import struct
from fastopen.workload import FastOpenOperation


class NetworkOperations:
    def __init__(self, root, timeout, poll):
        self.fastopen = FastOpenOperation(timeout, poll)
        self.libc = ctypes.CDLL(None, use_errno=True)
        self.libc.sendto.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_size_t,
                                    ctypes.c_int, ctypes.c_void_p, ctypes.c_uint]
        self.libc.sendto.restype = ctypes.c_ssize_t
        self.endpoints = json.loads((root / "endpoints.json").read_text())
        self.sockets = {}
        try:
            for name, endpoint in self.endpoints.items():
                family = socket.AF_INET6 if endpoint["version"] == 6 else socket.AF_INET
                kind = socket.SOCK_DGRAM if endpoint["protocol"] == "udp" else socket.SOCK_STREAM
                peer = socket.socket(family, kind)
                self.sockets[name] = peer
                peer.settimeout(timeout)
                peer.connect((endpoint["host"], endpoint["port"]))
                if kind == socket.SOCK_DGRAM:
                    self.sockets[name + "-unconnected"] = socket.socket(family, kind)
        except BaseException:
            self.close()
            raise

    def run(self, phase, results):
        for name, endpoint in self.endpoints.items():
            peer = self.sockets[name]
            destination = (endpoint["host"], endpoint["port"])
            actions = {
                "send": lambda data: peer.send(data),
                "write": lambda data: os.write(peer.fileno(), data),
            }
            if endpoint["protocol"] == "udp":
                unconnected = self.sockets[name + "-unconnected"]
                actions.update(
                    sendto=lambda data: unconnected.sendto(data, destination),
                    sendmsg=lambda data: unconnected.sendmsg([data], [], 0, destination),
                    unspecified=lambda data: self.send_unspecified(peer, unconnected, endpoint, data),
                )
            else:
                # TCP ignores this allowed alternative destination; enforce the real peer.
                control = self.endpoints[name.replace("protected-", "control-")]
                actions["sendmsg-address"] = lambda data: peer.sendmsg(
                    [data], [], 0, (control["host"], control["port"]),
                )
                if endpoint["fastopen"]:
                    actions["fastopen"] = lambda data: self.fastopen.send(endpoint, data)
            for operation, action in actions.items():
                key = f"{name}-{operation}"
                token = f"{phase}/{key}"
                try:
                    payload = (token + "\n").encode()
                    written = action(payload)
                    if written != len(payload):
                        raise AssertionError(f"partial local send: {key}: {written}")
                    results[key] = {"allowed": True, "peer": name, "token": token}
                except OSError as error:
                    results[key] = {"allowed": False, "errno": error.errno,
                                    "peer": name, "token": token}

    def send_unspecified(self, connected, unconnected, endpoint, data):
        address = bytes(2)
        peer = connected
        if endpoint["version"] == 4:
            address += (struct.pack("!H", endpoint["port"])
                        + socket.inet_pton(socket.AF_INET, endpoint["host"]) + bytes(8))
            peer = unconnected
        destination = ctypes.create_string_buffer(address)
        payload = ctypes.create_string_buffer(data)
        count = self.libc.sendto(peer.fileno(), payload, len(data), 0, destination, len(address))
        if count < 0:
            code = ctypes.get_errno()
            raise OSError(code, os.strerror(code))
        return count

    def close(self):
        for peer in self.sockets.values():
            peer.close()
        self.sockets.clear()
