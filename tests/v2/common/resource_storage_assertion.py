"""Cross-check real resource storage through SQL and public viewer output."""
from __future__ import annotations

from collections import Counter
from contextlib import closing
import json
from pathlib import Path
import sqlite3


METRIC_COLUMNS = (
    "cpu_percent_millis", "rss_kb", "virtual_memory_kb",
    "memory_current_bytes", "memory_peak_bytes", "memory_anon_bytes",
    "memory_file_bytes", "memory_swap_current_bytes", "cpu_usage_usec",
    "cpu_user_usec", "cpu_system_usec", "cpu_nr_throttled", "cpu_throttled_usec",
    "io_read_bytes", "io_write_bytes", "pids_current", "pids_peak", "process_rss_sum_kb",
)


def _connect(database: Path) -> sqlite3.Connection:
    return sqlite3.connect(database.resolve().as_uri() + "?mode=ro", uri=True)


def storage_counts(database: Path) -> tuple[int, int]:
    with closing(_connect(database)) as connection:
        tables = {row[0] for row in connection.execute("SELECT name FROM sqlite_master WHERE type='table'")}
        required = {
            "resource_host_samples", "resource_workload_samples", "resource_host_boots",
            "resource_trace_profiles", "resource_metadata_keys",
        }
        if "resource_samples" in tables or not required.issubset(tables):
            raise AssertionError(f"unexpected resource schema: missing={required - tables}, legacy={'resource_samples' in tables}")
        counts = connection.execute(
            "SELECT (SELECT count(*) FROM resource_host_samples), "
            "(SELECT count(*) FROM resource_workload_samples)"
        ).fetchone()
        for samples, profiles, key in (
            ("resource_workload_samples", "resource_trace_profiles", "trace_id"),
            ("resource_host_samples", "resource_host_boots", "boot_key"),
        ):
            if connection.execute(
                f"SELECT count(*) FROM {samples} s LEFT JOIN {profiles} p USING ({key}) WHERE p.{key} IS NULL"
            ).fetchone()[0]:
                raise AssertionError(f"{samples} contains samples without {profiles}")
        if connection.execute(
            "SELECT count(*) FROM resource_workload_samples WHERE extra IS NOT NULL AND typeof(extra) != 'blob'"
        ).fetchone()[0]:
            raise AssertionError("workload extra was not stored as BLOB")
    if sum(counts) == 0:
        raise AssertionError("resource storage received no real samples")
    return counts


def profile_snapshot(database: Path, trace_id: int) -> tuple[str, str, str]:
    with closing(_connect(database)) as connection:
        rows = connection.execute(
            "SELECT scope, subject, metadata FROM resource_trace_profiles WHERE trace_id=?", (trace_id,),
        ).fetchall()
    if len(rows) != 1:
        raise AssertionError(f"trace-{trace_id} expected one profile, got {len(rows)}")
    return rows[0]


def assert_host_columns(database: Path, samples: list[dict]) -> None:
    with closing(_connect(database)) as connection:
        rows = connection.execute(
            "SELECT lower(hex(b.boot_id)), s.observed_at_ns, s.cpu_percent_millis, "
            "s.memory_total_bytes, s.memory_available_bytes, s.memory_free_bytes, s.load_1, s.load_5, s.load_15 "
            "FROM resource_host_samples s JOIN resource_host_boots b USING (boot_key) "
            "ORDER BY s.observed_at_ns DESC,s.id DESC"
        ).fetchall()
    expected = [(
        sample["host_boot_id"].replace("-", "").lower(), int(sample["observed_at_unix_nanos"]),
        sample["cpu_percent_millis"], sample["memory_total_bytes"], sample["memory_available_bytes"],
        sample["memory_free_bytes"], *sample["load_average"],
    ) for sample in samples]
    unsigned_rows = [tuple(value & ((1 << 64) - 1) if isinstance(value, int) else value for value in row) for row in rows]
    if unsigned_rows != expected:
        raise AssertionError("host viewer rows differ from stored boot IDs, timestamps or metric columns")


def assert_restart_storage(
    database: Path, trace_id: int, before_profile: tuple[str, str, str],
    before_samples: list[dict], after_samples: list[dict],
) -> str:
    if profile_snapshot(database, trace_id) != before_profile:
        raise AssertionError(f"trace-{trace_id} profile changed after daemon restart")
    if not before_samples or len(after_samples) <= len(before_samples):
        raise AssertionError("restart did not preserve old samples and append new samples")
    def records(samples: list[dict]) -> Counter:
        return Counter(json.dumps(sample, sort_keys=True) for sample in samples)
    if records(before_samples) - records(after_samples):
        raise AssertionError("samples from before restart changed or disappeared")
    return f"profile unchanged; {len(before_samples)} pre-restart samples preserved field for field"


def assert_workload_columns(database: Path, trace_id: int, samples: list[dict]) -> str:
    host, workload = storage_counts(database)
    if not samples or any(sample["trace_id"] != trace_id for sample in samples):
        raise AssertionError(f"viewer returned no samples or another trace for trace-{trace_id}")
    with closing(_connect(database)) as connection:
        rows = connection.execute(
            "SELECT observed_at_ns, process_id, " + ", ".join(METRIC_COLUMNS)
            + " FROM resource_workload_samples WHERE trace_id=? ORDER BY observed_at_ns DESC,id DESC",
            (trace_id,),
        ).fetchall()
    expected = []
    for sample in samples:
        timestamp = sample["observed_at"]
        expected.append((
            timestamp["secs_since_epoch"] * 1_000_000_000 + timestamp["nanos_since_epoch"],
            sample["process"], *(sample["payload"][column] for column in METRIC_COLUMNS),
        ))
    unsigned_rows = [tuple(value & ((1 << 64) - 1) if isinstance(value, int) else value for value in row) for row in rows]
    if unsigned_rows != expected:
        raise AssertionError(f"trace-{trace_id} viewer rows differ from SQL columns, count, or order")
    return f"host={host}, workload={workload}; {len(rows)} viewer rows match all {len(METRIC_COLUMNS)} numeric columns"
