from __future__ import annotations

from contextlib import closing
from pathlib import Path
import sqlite3
import subprocess
import time

from tests.v2.common.core import TestCase, TestCaseInputs, TestResult, TestStatus
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.runner import TestingContextSingleton
from .constants import CASE_CONSTANTS as C


class ResourceStorageRetentionCase(TestCase):
    """Seed a backlog, then exercise production cleanup through real host writes."""

    def __init__(self, inputs: TestCaseInputs):
        self._inputs = inputs
        self._database = inputs.work_dir / "data/resources.sqlite"
        self._config = inputs.work_dir / "actraild.conf"
        self._log = inputs.work_dir / "log/retention-daemon.log"
        self._daemon: subprocess.Popen | None = None

    def _binary(self, name: str) -> Path:
        directory = self._inputs.bin_dir
        return (directory if directory.is_absolute() else self._inputs.repo / directory) / name

    def _prepare(self) -> None:
        work = self._inputs.work_dir
        for directory in ("data", "log", "run", "plugins"):
            (work / directory).mkdir(parents=True, exist_ok=True)
        patch = work / "retention.patch.toml"
        PatchBuilder().with_work_dir(work).with_values({
            "storage": {"retention": {"enabled": False}},
            "capture": {"profile_name": "resource-storage-retention", "capabilities": ["resource-metrics"],
                        "opportunistic_capabilities": [], "disabled_capabilities": []},
            "ebpf": {"enabled": "false"},
            "payload": {"tls": {"enabled": False}},
            "plugins": {"startup": {"enabled": False, "load": []}},
            "access_control": {
                "file": {operation: {"enabled": False} for operation in ("open", "mkdir", "rmdir")},
                "command": {"exec": {"enabled": False}}, "network": {"connect": {"enabled": False}},
            },
            "resource_profiling": {
                "enabled": True,
                "collection": {"host": {"enabled": True, "interval_ms": C.sample_interval_ms},
                               "workload": {"enabled": True, "mode": "procfs", "interval_ms": C.sample_interval_ms}},
                "storage": {"database": {"sqlite": {
                    "path": str(self._database),
                    "retention": {"enabled": True, "max_age_seconds": C.max_age_seconds,
                                  "sweep_interval_seconds": C.sweep_interval_seconds, "max_samples_per_sweep": C.batch_size},
                }}},
            },
        }).build().save(patch)
        command = subprocess.run(
            [str(self._binary("actrailctl")), "--config", str(self._config), "init", "--force", "--patch", str(patch)],
            cwd=self._inputs.repo, capture_output=True, text=True, timeout=C.command_timeout_seconds,
        )
        if command.returncode:
            raise RuntimeError(f"retention config initialization failed: {command.stdout}{command.stderr}")

    def _seed(self) -> tuple:
        now = time.time_ns()
        old = now - C.max_age_seconds * 2 * 1_000_000_000
        schema = self._inputs.repo / "crates/core/resource_profiling/src/storage/database/sqlite/schema.sql"
        with closing(sqlite3.connect(self._database)) as connection:
            connection.executescript(schema.read_text(encoding="utf-8"))
            connection.execute("CREATE TABLE resource_samples(id INTEGER PRIMARY KEY, payload TEXT)")
            connection.execute("INSERT INTO resource_samples(payload) VALUES ('obsolete prototype sample')")
            connection.executemany("INSERT INTO resource_host_boots VALUES (?,?)",
                                   [(boot, boot.to_bytes(16, "big")) for boot in (901, 902, 903)])
            connection.executemany("INSERT INTO resource_trace_profiles VALUES (?,?,?,?)", [
                (trace, "trace", f"trace-{trace}", '{"ownership":"managed"}')
                for trace in (900001, 900002, 900003)
            ])
            connection.execute("INSERT INTO resource_metadata_keys VALUES (1,'retention-preserve-key')")
            connection.executemany(
                "INSERT INTO resource_host_samples "
                "(id,boot_key,observed_at_ns,memory_total_bytes,memory_available_bytes,memory_free_bytes,load_1,load_5,load_15) "
                "VALUES (?,901,?,1000,500,100,0,0,0)",
                ((-index, old) for index in range(1, C.expired_per_table + 1)),
            )
            connection.executemany(
                "INSERT INTO resource_workload_samples(id,trace_id,observed_at_ns,process_id,flags) VALUES (?,900001,?,1,21)",
                ((-index, old) for index in range(1, C.expired_per_table + 1)),
            )
            # One trace/boot has both old and fresh rows: its profile must survive.
            connection.execute("INSERT INTO resource_workload_samples(id,trace_id,observed_at_ns,process_id,flags) VALUES (0,900002,?,1,21)", (old,))
            connection.execute(
                "INSERT INTO resource_host_samples(id,boot_key,observed_at_ns,memory_total_bytes,memory_available_bytes,memory_free_bytes,load_1,load_5,load_15) "
                "VALUES (0,902,?,1000,500,100,0,0,0)", (old,),
            )
            for index, (boot, trace) in enumerate(((902, 900002), (903, 900003)), 1):
                connection.execute(
                    "INSERT INTO resource_workload_samples(id,trace_id,observed_at_ns,process_id,flags,cpu_usage_usec,memory_current_bytes) "
                    "VALUES (?,?,?,1,21,123456,654321)", (index, trace, now),
                )
                connection.execute(
                    "INSERT INTO resource_host_samples(id,boot_key,observed_at_ns,memory_total_bytes,memory_available_bytes,memory_free_bytes,load_1,load_5,load_15) "
                    "VALUES (?,?,?,1000,500,100,0.1,0.2,0.3)", (index, boot, now),
                )
            connection.commit()
            return self._fresh_rows(connection)

    @staticmethod
    def _fresh_rows(connection: sqlite3.Connection) -> tuple:
        return tuple(connection.execute(f"SELECT * FROM {table} WHERE id IN (1,2) ORDER BY id").fetchall()
                     for table in ("resource_host_samples", "resource_workload_samples"))

    def _wait_for_daemon(self) -> None:
        assert self._daemon is not None
        deadline = time.monotonic() + C.command_timeout_seconds
        last_reply = "control request has not succeeded"
        while time.monotonic() < deadline:
            if self._daemon.poll() is not None:
                raise RuntimeError(f"retention daemon failed during initialization: {self._log.read_text()[-2000:]}")
            try:
                reply = subprocess.run(
                    [str(self._binary("actrailctl")), "--config", str(self._config), "list-traces"],
                    cwd=self._inputs.repo, capture_output=True, text=True,
                    timeout=C.control_request_timeout_seconds,
                )
            except subprocess.TimeoutExpired as error:
                last_reply = str(error)
            else:
                if reply.returncode == 0:
                    return
                last_reply = (reply.stderr or reply.stdout)[-2000:]
            time.sleep(C.poll_seconds)
        raise TimeoutError(f"retention daemon did not become ready: {last_reply}; {self._log.read_text()[-2000:]}")

    def run(self, context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        current = "setup"
        try:
            self._prepare()
            fresh = self._seed()
            started = time.monotonic()
            with self._log.open("w", encoding="utf-8") as log:
                self._daemon = subprocess.Popen(
                    [str(self._binary("actraild")), "--config", str(self._config), "run"],
                    cwd=self._inputs.repo, stdout=log, stderr=subprocess.STDOUT,
                )
            current = "startup"
            context.report_progress(current, "waiting for a successful daemon control request before reading resource storage")
            # Storage initialization (including WAL setup) precedes control service readiness.
            self._wait_for_daemon()
            current = "expired_samples"
            context.report_progress(current, "waiting for real host writes to clear both resource backlogs")
            with closing(sqlite3.connect(self._database.resolve().as_uri() + "?mode=ro", uri=True)) as connection:
                while True:
                    remaining = tuple(connection.execute(f"SELECT count(*) FROM {table} WHERE id<=0").fetchone()[0]
                                      for table in ("resource_host_samples", "resource_workload_samples"))
                    if remaining == (0, 0):
                        break
                    if self._daemon.poll() is not None or time.monotonic() - started > C.cleanup_timeout_seconds:
                        raise AssertionError(f"expired rows remain: host/workload={remaining}; {self._log.read_text()[-2000:]}")
                    time.sleep(C.poll_seconds)
                writes = connection.execute("SELECT count(*) FROM resource_host_samples WHERE id>2").fetchone()[0]
                results[current] = TestResult(TestStatus.PASSED, f"{C.expired_per_table + 1} expired rows per table cleared in {time.monotonic() - started:.2f}s across {writes} host writes with batch={C.batch_size}")
                current = "fresh_samples"
                if self._fresh_rows(connection) != fresh:
                    raise AssertionError("retention changed or deleted fresh resource samples")
                results[current] = TestResult(TestStatus.PASSED, "all four fresh sample rows preserved field for field")
                current = "profiles_and_boots"
                profiles = connection.execute("SELECT trace_id FROM resource_trace_profiles ORDER BY trace_id").fetchall()
                boots = connection.execute("SELECT boot_key FROM resource_host_boots WHERE boot_key IN (901,902,903) ORDER BY boot_key").fetchall()
                if profiles != [(900002,), (900003,)] or boots != [(902,), (903,)]:
                    raise AssertionError(f"profile/boot reclamation disagrees with surviving samples: {profiles}, {boots}")
                results[current] = TestResult(TestStatus.PASSED, "expired-only profile/boot reclaimed; mixed and fresh profile/boot retained")
                current = "metadata_keys"
                if connection.execute("SELECT key_id,key FROM resource_metadata_keys").fetchall() != [(1, "retention-preserve-key")]:
                    raise AssertionError("retention reclaimed metadata dictionary keys")
                results[current] = TestResult(TestStatus.PASSED, "metadata dictionary preserved")
                current = "legacy_table"
                if connection.execute("SELECT name FROM sqlite_master WHERE name='resource_samples'").fetchone():
                    raise AssertionError("opening the column store did not drop the legacy sample table")
                results[current] = TestResult(TestStatus.PASSED, "legacy prototype sample table dropped")
                if self._daemon.poll() is not None:
                    raise AssertionError(f"retention daemon exited unexpectedly: {self._log.read_text()[-2000:]}")
        except Exception as error:
            results[current] = TestResult(TestStatus.FAILED, str(error))
        finally:
            if self._daemon is not None and self._daemon.poll() is None:
                self._daemon.terminate()
                try:
                    self._daemon.wait(timeout=C.stop_timeout_seconds)
                except subprocess.TimeoutExpired:
                    self._daemon.kill()
                    self._daemon.wait(timeout=C.kill_timeout_seconds)
                    results["cleanup"] = TestResult(TestStatus.FAILED, "retention daemon required SIGKILL")
        return TestResult(TestStatus.COMPOSITE, "resource SQLite retention", results)
