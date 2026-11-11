#!/usr/bin/env python3
"""Exercise retained descriptors inside a real Xiaoo Bash tool invocation."""

from __future__ import annotations

import argparse
import json
import os
import socket
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from workload import AccessWorkload
from mapping import MappingOperations
from network import NetworkOperations
from removal import RemovalOperations
from metadata import MetadataOperations


class ContinuedWorkload:
    def __init__(self, arguments):
        self.arguments = arguments
        self.root = arguments.root

    def run(self):
        processes = AccessWorkload(self.arguments)._processes()
        descriptors = {name: os.open(self.root / name, os.O_RDWR)
                       for name in ("protected", "control")}
        mappings = None
        network = None
        removal = None
        try:
            mappings = MappingOperations(descriptors)
            network = NetworkOperations(self.root, self.arguments.timeout, self.arguments.poll)
            removal = RemovalOperations(self.root)
            metadata = MetadataOperations(self.root, descriptors)
            for phase in ("initial", "deny", "restore"):
                (self.root / f"ready-{phase}").touch()
                self.wait(self.root / f"go-{phase}")
                result = {"processes": processes, "operations": {}}
                for name, descriptor in descriptors.items():
                    for operation, action in (
                        ("read", lambda: os.pread(descriptor, 1, 0).decode()),
                        ("write", lambda: os.pwrite(descriptor, phase.encode(), 0)),
                    ):
                        try:
                            result["operations"][f"{name}-{operation}"] = {
                                "allowed": True, "value": action(),
                            }
                        except OSError as error:
                            result["operations"][f"{name}-{operation}"] = {
                                "allowed": False, "errno": error.errno,
                            }
                mappings.run(result["operations"])
                network.run(phase, result["operations"])
                removal.run(result["operations"])
                metadata.run(phase, result["operations"])
                self.synthetic_descriptors()
                destination = self.root / f"result-{phase}.json"
                temporary = destination.with_suffix(".tmp")
                temporary.write_text(json.dumps(result))
                temporary.rename(destination)
            print("ACTRAIL_CONTINUED_ACCESS_DONE")
        finally:
            if mappings is not None:
                mappings.close()
            if network is not None:
                network.close()
            if removal is not None:
                removal.close()
            for descriptor in descriptors.values():
                os.close(descriptor)

    def wait(self, path):
        deadline = time.monotonic() + self.arguments.timeout
        while not path.exists():
            if time.monotonic() >= deadline:
                raise TimeoutError(f"parent did not release {path}")
            time.sleep(self.arguments.poll)

    @staticmethod
    def synthetic_descriptors():
        read_fd, write_fd = os.pipe()
        try:
            os.write(write_fd, b"p")
            assert os.read(read_fd, 1) == b"p"
        finally:
            os.close(read_fd)
            os.close(write_fd)
        left, right = socket.socketpair()
        with left, right:
            left.sendall(b"s")
            assert right.recv(1) == b"s"
        descriptor = os.memfd_create("actrail-continued")
        try:
            os.write(descriptor, b"m")
            assert os.pread(descriptor, 1, 0) == b"m"
        finally:
            os.close(descriptor)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, required=True)
    parser.add_argument("--xiaoo", type=Path, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--poll", type=float, required=True)
    ContinuedWorkload(parser.parse_args()).run()
