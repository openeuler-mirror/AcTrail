"""Identical log-based completion window for SQLite and NoOp measurements."""

import re
import time
import tomllib
from pathlib import Path

from scripts.bench.payload.runtime import CollectionRuntime


class StorageCpuRuntime(CollectionRuntime):
    def __init__(self, work_dir, bin_dir, patch, settings, backend):
        super().__init__(work_dir, bin_dir, patch, settings)
        self.backend = backend
        self.completed_trace = 0

    def start(self):
        super().start()
        effective = tomllib.loads(self.config.read_text())
        if effective["storage"]["backend"] != self.backend:
            raise RuntimeError("effective backend differs from CPU comparison selection")

    def mark(self):
        self.log.seek(0, 2)
        return self.completed_trace

    def drain(self, previous_trace):
        started_at = time.monotonic()
        deadline = started_at + self.settings["drain_timeout_seconds"]
        launched, completed = set(), set()
        fragment = ""
        while time.monotonic() < deadline:
            self.cpu.read_ms()
            fragment += self.log.read()
            lines = fragment.split("\n")
            fragment = lines.pop()
            for line in lines:
                for pattern, target in (
                    (r"agent_launch started trace_id=trace-(\d+)\b", launched),
                    (r"trace_finalization completed trace_id=trace-(\d+)\b", completed),
                ):
                    match = re.search(pattern, line)
                    if match:
                        target.add(int(match[1]))
            if len(launched) > 1 or any(trace <= previous_trace for trace in launched):
                raise RuntimeError(f"unexpected launch facts in CPU cell: {sorted(launched)}")
            if launched and launched <= completed:
                self.completed_trace = next(iter(launched))
                return {"drain_ms": (time.monotonic() - started_at) * 1000,
                        "lifecycle": {"trace_id": self.completed_trace,
                                      "launched": True, "finalized": True,
                                      "source": "isolated daemon log"}}
            time.sleep(self.settings["poll_seconds"])
        raise TimeoutError(f"log finalization timeout: launched={launched}, completed={completed}")

    def evidence(self, previous_trace: int, kind: str, directory: Path):
        if self.backend == "sqlite":
            evidence = super().evidence(previous_trace, kind, directory)
            if evidence["traces"][0][0] != self.completed_trace:
                raise RuntimeError("stored trace differs from log completion")
            return evidence
        if self.database.exists():
            raise RuntimeError("NoOp CPU runtime unexpectedly created SQLite storage")
        return {"backend": "noop", "trace_id": self.completed_trace,
                "launched": True, "finalized": True, "database_created": False,
                "online_capture_coverage": "not inferred from CPU run; use independent acceptance",
                "workload_validation": "real MaaS requests and actual tool outputs in workload_result"}
