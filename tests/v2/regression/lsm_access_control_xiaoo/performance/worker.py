from __future__ import annotations

import argparse
import errno
import json
import os
import resource
import socket
import statistics
import time
from pathlib import Path

from ..workload import AccessWorkload

from .constants import CASE_CONSTANTS


class Worker:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        self._daemon = int(args.daemon_pid.read_text().strip())
        self._ticks = int(os.sysconf("SC_CLK_TCK"))

    def _daemon_ticks(self) -> int:
        # stat fields after the last ')' begin with field 3 (state).
        fields = (Path("/proc") / str(self._daemon) / "stat").read_text().rsplit(")", 1)[1].split()
        return int(fields[11]) + int(fields[12])

    def _connect(self, decision: str) -> None:
        port = self.args.allow_port if decision == "allow" else self.args.deny_port
        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as client:
            client.settimeout(self.args.timeout)
            try:
                client.connect((CASE_CONSTANTS.loopback_host, port))
            except OSError as error:
                if decision != "deny" or error.errno not in (errno.EACCES, errno.EPERM):
                    raise
            else:
                if decision == "deny":
                    raise AssertionError("denied endpoint was reachable")

    def _round(self, decision: str, number: int) -> dict:
        daemon_start = self._daemon_ticks()
        usage_start = resource.getrusage(resource.RUSAGE_SELF)
        durations = []
        started = time.perf_counter_ns()
        for _ in range(self.args.count):
            before = time.perf_counter_ns()
            self._connect(decision)
            durations.append(time.perf_counter_ns() - before)
        elapsed = (time.perf_counter_ns() - started) / 1e9
        usage_end = resource.getrusage(resource.RUSAGE_SELF)
        daemon_ticks = self._daemon_ticks() - daemon_start
        ordered = sorted(durations)
        return {
            "decision": decision, "round": number, "count": self.args.count,
            "elapsed_seconds": elapsed, "operations_per_second": self.args.count / elapsed,
            "mean_latency_us": statistics.mean(durations) / 1000,
            "median_latency_us": statistics.median(durations) / 1000,
            "p95_latency_us": ordered[min(len(ordered) - 1, int(len(ordered) * 0.95))] / 1000,
            "worker_user_cpu_seconds": usage_end.ru_utime - usage_start.ru_utime,
            "worker_system_cpu_seconds": usage_end.ru_stime - usage_start.ru_stime,
            "worker_voluntary_context_switches": usage_end.ru_nvcsw - usage_start.ru_nvcsw,
            "worker_involuntary_context_switches": usage_end.ru_nivcsw - usage_start.ru_nivcsw,
            "daemon_cpu_ticks": daemon_ticks, "daemon_cpu_seconds": daemon_ticks / self._ticks,
            "latency_ns": durations,
        }

    def run(self) -> None:
        probe = AccessWorkload(self.args)
        processes = probe._processes()
        expected_seccomp = 2 if self.args.backend == "seccomp" else 0
        for process in processes:
            if not process["threads"] or any(
                thread["seccomp"] != expected_seccomp for thread in process["threads"]
            ):
                raise AssertionError(f"unexpected control mechanism: {process}")
        daemon_fds = probe._descriptors(self._daemon)
        if any("fanotify" in descriptor for descriptor in daemon_fds):
            raise AssertionError("benchmark daemon installed fanotify")
        if self.args.backend == "lsm" and daemon_fds:
            raise AssertionError(f"LSM daemon has seccomp descriptors: {daemon_fds}")
        for decision in ("allow", "deny"):
            for _ in range(self.args.warmup):
                self._connect(decision)
        all_start = self._daemon_ticks()
        rounds = []
        for number in range(1, self.args.rounds + 1):
            # Alternate decision order to reduce within-run order bias.
            for decision in (("allow", "deny") if number % 2 else ("deny", "allow")):
                rounds.append(self._round(decision, number))
        result = {
            "backend": self.args.backend, "pid": os.getpid(), "processes": processes,
            "daemon_fds": daemon_fds, "clock_ticks_per_second": self._ticks,
            "warmup_per_decision": self.args.warmup,
            "daemon_cpu_ticks_measured_rounds": self._daemon_ticks() - all_start,
            "rounds": rounds,
        }
        self.args.results.write_text(json.dumps(result, indent=2) + "\n")
        print("ACTRAIL_NETWORK_BENCHMARK_COMPLETE", flush=True)


