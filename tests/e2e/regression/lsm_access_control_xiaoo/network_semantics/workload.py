#!/usr/bin/env python3
"""Exercise IP protocols and local/remote operations inside real Xiaoo."""
import argparse
import json
import os
from pathlib import Path
import socket
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from workload import AccessWorkload
sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "continued"))
from network import NetworkOperations


class NetworkWorkload:
    phases = ("empty", "tcp", "udp", "connect", "bind", "listen", "restore")

    def __init__(self, arguments):
        self.arguments = arguments
        self.root = arguments.root

    def run(self):
        processes = AccessWorkload(self.arguments)._processes()
        network = NetworkOperations(self.root, self.arguments.timeout, self.arguments.poll)
        listeners = {}
        try:
            # Bind fresh sockets before publication, so listen is tested independently of bind.
            for phase in self.phases:
                for version, family, host in ((4, socket.AF_INET, "127.0.0.3"), (6, socket.AF_INET6, "::1")):
                    peer = socket.socket(family, socket.SOCK_STREAM)
                    peer.bind((host, 0))
                    listeners[phase, version] = peer
            (self.root / "locals.json").write_text(json.dumps({str(version): listeners["listen", version].getsockname()[1] for version in (4, 6)}))
            for phase in self.phases:
                (self.root / f"ready-{phase}").touch()
                self.wait(self.root / f"go-{phase}")
                operations = {}
                network.run(phase, operations)
                for name, endpoint in network.endpoints.items():
                    family = socket.AF_INET6 if endpoint["version"] == 6 else socket.AF_INET
                    kind = socket.SOCK_STREAM if endpoint["protocol"] == "tcp" else socket.SOCK_DGRAM
                    with socket.socket(family, kind) as peer:
                        peer.settimeout(self.arguments.timeout)
                        destination = (endpoint["host"], endpoint["port"], 0, 1) if endpoint["version"] == 6 else (endpoint["host"], endpoint["port"])
                        self.record(operations, f"connect-{name}", lambda: peer.connect(destination))
                        if operations[f"connect-{name}"]["allowed"]:
                            assert peer.getpeername()[:2] == (endpoint["host"], endpoint["port"])
                for version, family, host in ((4, socket.AF_INET, "127.0.0.3"), (6, socket.AF_INET6, "::1")):
                    for protocol, kind in (("tcp", socket.SOCK_STREAM), ("udp", socket.SOCK_DGRAM)):
                        with socket.socket(family, kind) as peer:
                            key = f"bind-{protocol}{version}"
                            self.record(operations, key, lambda: peer.bind((host, 0)))
                            assert bool(peer.getsockname()[1]) == operations[key]["allowed"]
                    peer = listeners[phase, version]
                    key = f"listen-tcp{version}"
                    self.record(operations, key, peer.listen)
                    assert bool(peer.getsockopt(socket.SOL_SOCKET, socket.SO_ACCEPTCONN)) == operations[key]["allowed"]
                destination = self.root / f"result-{phase}.json"
                temporary = destination.with_suffix(".tmp")
                temporary.write_text(json.dumps({"processes": processes, "operations": operations}))
                temporary.rename(destination)
            print("ACTRAIL_NETWORK_SEMANTICS_DONE")
        finally:
            network.close()
            for peer in listeners.values():
                peer.close()

    @staticmethod
    def record(operations, key, action):
        try:
            action()
            operations[key] = {"allowed": True}
        except OSError as error:
            operations[key] = {"allowed": False, "errno": error.errno}

    def wait(self, path):
        deadline = time.monotonic() + self.arguments.timeout
        while not path.exists():
            if time.monotonic() >= deadline:
                raise TimeoutError(f"parent did not release {path}")
            time.sleep(self.arguments.poll)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--xiaoo", type=Path, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--poll", type=float, required=True)
    NetworkWorkload(parser.parse_args()).run()
