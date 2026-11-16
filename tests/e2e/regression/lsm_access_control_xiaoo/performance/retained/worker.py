"""Measure repeated operations on files and sockets opened before timing."""

import json
import os
import socket

from ...workload import AccessWorkload
from ..worker import Worker


class RetainedWorker(Worker):
    def _connect(self, operation):
        # Reuse the existing round timer and CPU accounting without opening FDs.
        self.actions[operation]()

    def run(self):
        processes = AccessWorkload(self.args)._processes()
        if any(thread["seccomp"] for process in processes for thread in process["threads"]):
            raise AssertionError("retained benchmark requires Seccomp=0")
        descriptor = os.open(self.args.file, os.O_RDWR)
        try:
            with socket.create_connection(("127.0.0.2", self.args.port), self.args.timeout) as peer:
                self.actions = {
                    "read": lambda: os.pread(descriptor, 1, 0),
                    "write": lambda: os.pwrite(descriptor, b"x", 0),
                    "send": lambda: peer.sendall(b"x" * 64),
                }
                for action in self.actions.values():
                    for _ in range(self.args.warmup):
                        action()
                rounds = []
                for number in range(1, self.args.rounds + 1):
                    order = tuple(self.actions) if number % 2 else tuple(reversed(self.actions))
                    for operation in order:
                        rounds.append(self._round(operation, number))
                if os.pread(descriptor, 1, 0) != b"x":
                    raise AssertionError("writes did not reach the real file")
                self.args.results.write_text(json.dumps({
                    "backend": self.args.backend, "processes": processes,
                    "count": self.args.count, "rounds": rounds,
                    "sent_bytes": (self.args.warmup + self.args.rounds * self.args.count) * 64,
                }, indent=2) + "\n")
        finally:
            os.close(descriptor)
        print("ACTRAIL_RETAINED_BENCHMARK_COMPLETE")
