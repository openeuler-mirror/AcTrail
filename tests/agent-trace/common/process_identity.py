"""Persisted logical process identity checks for E2E cases."""

from __future__ import annotations

import re
import sqlite3
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class StoredTraceRoot:
    process_id: int
    host_pid: int | None
    namespace_pids: frozenset[int]

    @classmethod
    def load(cls, storage: Path, trace_id: int) -> StoredTraceRoot:
        with sqlite3.connect(storage) as connection:
            rows = connection.execute(
                """
                SELECT trace.root_process_id, process.host_pid, alias.namespace_pid
                FROM traces trace
                JOIN processes process
                  ON process.process_id = trace.root_process_id
                LEFT JOIN process_namespace_aliases alias
                  ON alias.process_id = trace.root_process_id
                WHERE trace.trace_id = ?
                """,
                (trace_id,),
            ).fetchall()
        if not rows:
            raise RuntimeError(f"trace-{trace_id} has no persisted root process")
        process_ids = {int(row[0]) for row in rows}
        host_pids = {int(row[1]) for row in rows if row[1] is not None}
        if len(process_ids) != 1 or len(host_pids) > 1:
            raise RuntimeError(
                f"trace-{trace_id} root process rows are inconsistent: {rows}"
            )
        return cls(
            process_id=next(iter(process_ids)),
            host_pid=next(iter(host_pids), None),
            namespace_pids=frozenset(int(row[2]) for row in rows if row[2] is not None),
        )

    def require_summary(self, summary: str) -> None:
        match = re.search(r"\broot_process_id=(\d+)\b", summary)
        if match is None:
            raise RuntimeError(f"trace summary has no root_process_id\n{summary}")
        summary_process_id = int(match.group(1))
        if summary_process_id != self.process_id:
            raise RuntimeError(
                "trace summary root process does not match persisted root: "
                f"summary={summary_process_id} persisted={self.process_id}\n{summary}"
            )

    def require_namespace_pid(self, namespace_pid: int) -> None:
        if namespace_pid not in self.namespace_pids:
            raise RuntimeError(
                f"root process {self.process_id} has no namespace PID {namespace_pid}; "
                f"host_pid={self.host_pid} namespace_pids={sorted(self.namespace_pids)}"
            )

    def facts(self, workload_pid: int) -> list[str]:
        return [
            f"root_process_id={self.process_id}",
            f"root_host_pid={self.host_pid if self.host_pid is not None else 'unresolved'}",
            f"root_namespace_pid={workload_pid}",
        ]
