from __future__ import annotations

import errno
import json
import os
import shutil
import signal
import sqlite3
import subprocess
import time
from pathlib import Path
from typing import Callable

from tests.v2.common.core import TestCase, TestCaseInputs, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.resource_storage_assertion import (
    assert_restart_storage, assert_workload_columns, profile_snapshot,
)
from .constants import CASE_CONSTANTS
from .resource_api import assert_resource_api

CGROUP_ROOT = Path("/sys/fs/cgroup")
WORKLOAD = Path(__file__).resolve().parent / "workload.py"
BURN = "import time\nend = time.time() + {seconds}\nwhile time.time() < end: pass\n"
SLEEP = "import time\ntime.sleep({seconds})\n"


class ResourceMetricsCgroupV1Case(TestCase):
    """Acceptance for daemon-owned cgroup-v1 trace scopes, run on hosts without unified v2."""

    def __init__(self, inputs: TestCaseInputs) -> None:
        self._inputs = inputs
        # A per-run root keeps this case away from a deployed daemon's `actrail/`.
        self._root_name = f"actrail-e2e-{os.getpid()}"
        self._config = inputs.work_dir / "actraild.conf"
        self._socket = inputs.work_dir / "run/control.sock"
        self._database = inputs.work_dir / "data/actrail.sqlite"
        self._daemon: subprocess.Popen[bytes] | None = None
        self._hierarchies: list[Path] = []

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        problem = self._prerequisite_problem()
        if problem is not None:
            return TestResult(TestStatus.SKIPPED, problem)
        results: dict[str, TestResult] = {}
        current = "setup"
        try:
            self._prepare_config()
            self._start_daemon()
            self._assert_hierarchy()
            for current, check in (
                ("exact", self._run_exact_case),
                ("isolation", self._run_isolation_case),
                ("restart", self._run_restart_case),
                ("timeout", self._run_timeout_case),
                ("lost", self._run_lost_scope_case),
                ("column_storage", lambda: assert_workload_columns(
                    self._config.parent / "actrail-resources.sqlite", 1, self._resource_samples(1),
                )),
                ("resource_api", lambda: assert_resource_api(
                    self._inputs, self._config, {trace: self._resource_samples(trace) for trace in (1, 4)},
                )),
            ):
                test_context.report_progress(current, f"checking managed cgroup-v1 {current}")
                results[current] = TestResult(TestStatus.PASSED, check())
            return TestResult(
                TestStatus.COMPOSITE, "cgroup-v1 resource metrics", results,
            )
        except Exception as error:
            results[current] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(TestStatus.COMPOSITE, "cgroup-v1 resource metrics", results)
        finally:
            self._stop_daemon()
            self._remove_roots()

    def cleanup(self, test_context: TestingContextSingleton) -> TestResult | None:
        del test_context
        self._stop_daemon()
        self._remove_roots()
        return None

    def _prerequisite_problem(self) -> str | None:
        if os.geteuid() != 0:
            return "resource_metrics_cgroup requires root to create v1 cgroups"
        mounts = self._v1_mounts()
        if "cpuacct" not in mounts or "memory" not in mounts:
            return "cgroup v1 cpuacct and memory hierarchies are unavailable"
        if (CGROUP_ROOT / "cgroup.controllers").is_file():
            return "host uses unified cgroup v2; managed v2 takes precedence"
        self._hierarchies = sorted(set(mounts.values()))
        binaries = [self._binary(name) for name in ("actraild", "actrailctl", "actrailviewer", "actrailweb")]
        missing = [str(path) for path in binaries if not path.is_file()]
        if shutil.which("python3") is None:
            missing.append("python3")
        if missing:
            return "missing required commands or binaries: " + ", ".join(missing)
        return None

    @staticmethod
    def _v1_mounts() -> dict[str, Path]:
        mounts: dict[str, Path] = {}
        for line in Path("/proc/self/mountinfo").read_text(encoding="utf-8").splitlines():
            prefix, _, suffix = line.partition(" - ")
            fields, options = prefix.split(), suffix.split()
            if len(options) < 3 or options[0] != "cgroup" or fields[3] != "/":
                continue
            for controller in options[2].split(","):
                if controller in ("cpuacct", "memory", "pids", "blkio"):
                    mounts.setdefault(controller, Path(fields[4]))
        return mounts

    def _prepare_config(self) -> None:
        for directory in ("run", "data", "log", "export", "plugins"):
            (self._inputs.work_dir / directory).mkdir(parents=True, exist_ok=True)
        patch = self._inputs.work_dir / "operator.patch.toml"
        patch.write_text(self._operator_patch(), encoding="utf-8")
        self._command(
            (
                str(self._binary("actrailctl")),
                "--config",
                str(self._config),
                "init",
                "--force",
                "--patch",
                str(patch),
            )
        )

    def _operator_patch(self) -> str:
        work = self._inputs.work_dir
        return f'''[control]
socket_path = "{self._socket}"
pid_file = "{work / 'run/actraild.pid'}"
log_path = "{work / 'log/actraild.log'}"

[control.finalization]
poll_interval_ms = 25
settle_delay_ms = 25

[storage.sqlite]
path = "{self._database}"

[storage.retention]
enabled = false

[export.snapshot]
directory = "{work / 'export'}"

[plugins.discovery]
directory = "{work / 'plugins'}"

[plugins.startup]
enabled = false
load = []

[access_control.file.open]
enabled = false

[access_control.command.exec]
enabled = false

[access_control.network.connect]
enabled = false

[capture]
profile_name = "resource-metrics-cgroup-v1"
capabilities = ["resource-metrics"]
opportunistic_capabilities = []
disabled_capabilities = []

[payload.tls]
enabled = false
sync_event_socket_path = "{work / 'run/tls-sync.sock'}"

[resource_profiling]
enabled = true

[resource_profiling.collection.workload]
mode = "cgroup"
interval_ms = 50

[resource_profiling.collection.workload.cgroup]
mode = "v1"

[resource_profiling.collection.workload.cgroup.v1.managed]
root_name = "{self._root_name}"
recovery_scan_limit = 64
finalization_timeout_ms = 1000
'''

    def _start_daemon(self) -> None:
        self._daemon = subprocess.Popen(
            (str(self._binary("actraild")), "--config", str(self._config), "run"),
            cwd=self._inputs.repo,
            stdout=(self._inputs.work_dir / "log/daemon.out").open("ab"),
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        self._wait_until(self._doctor_ok, "daemon did not become ready", timeout=30)

    def _restart_daemon(self, *, stopped_pid_signal: int = signal.SIGKILL) -> None:
        assert self._daemon is not None
        self._daemon.send_signal(stopped_pid_signal)
        self._daemon.wait(timeout=10)
        self._start_daemon()

    def _stop_daemon(self) -> None:
        if self._daemon is None or self._daemon.poll() is not None:
            return
        self._daemon.terminate()
        try:
            self._daemon.wait(timeout=15)
        except subprocess.TimeoutExpired:
            self._daemon.kill()
            self._daemon.wait(timeout=5)

    def _assert_hierarchy(self) -> None:
        for mount in self._hierarchies:
            traces = mount / self._root_name / "traces"
            if not traces.is_dir():
                raise AssertionError(f"managed v1 root was not created: {traces}")
        assert self._daemon is not None
        membership = Path(f"/proc/{self._daemon.pid}/cgroup").read_text(encoding="utf-8")
        if f"/{self._root_name}/" in membership:
            raise AssertionError(f"daemon moved itself into the managed root: {membership!r}")

    def _run_exact_case(self) -> str:
        process = self._launch_workload("exact", child_seconds=0.8, parent_seconds=0.2)
        self._wait_until(
            lambda: (self._inputs.work_dir / "child-cgroup-exact").exists(),
            "exact workload did not start",
        )
        process.communicate(timeout=8)
        if process.returncode != 0:
            raise AssertionError("exact workload launch failed")
        self._wait_for_trace(1, "finalized")
        self._wait_until(
            lambda: not any(path.exists() for path in self._scope_replicas(1)),
            "empty v1 scope cleanup did not finish in every hierarchy",
        )
        payloads = self._resource_payloads(1)
        self._assert_launch_membership("exact")
        self._assert_final(payloads, coverage="exact")
        peak = max(payload["memory_peak_bytes"] or 0 for payload in payloads)
        if peak < 48 * 1024 * 1024:
            raise AssertionError(f"kernel memory peak was unexpectedly low: {peak}")
        if "pids" in self._v1_mounts() and max(
            payload["pids_peak"] or 0 for payload in payloads
        ) < 2:
            raise AssertionError("kernel pids.peak did not observe the descendant")
        return f"events={len(payloads)},peak={peak}"

    def _run_isolation_case(self) -> str:
        """An idle trace must not be charged for untraced or concurrent work."""
        noise = [
            subprocess.Popen((shutil.which("python3") or "python3", "-c", BURN.format(seconds=3)))
            for _ in range(2)
        ]
        try:
            busy = self._launch_code("busy", BURN.format(seconds=2.5))
            idle = self._launch_code("idle", SLEEP.format(seconds=2.5))
            busy.communicate(timeout=15)
            idle.communicate(timeout=15)
        finally:
            for process in noise:
                process.wait(timeout=15)
        # Both launches race for trace IDs 2 and 3; resolve them by name.
        busy_id, idle_id = self._trace_id("busy"), self._trace_id("idle")
        self._wait_for_trace(busy_id, "finalized")
        self._wait_for_trace(idle_id, "finalized")
        busy = self._final_cpu_seconds(self._resource_payloads(busy_id))
        idle = self._final_cpu_seconds(self._resource_payloads(idle_id))
        # Untraced noise burns 6 CPU-seconds in the same session meanwhile.
        if busy < 2.0:
            raise AssertionError(f"busy trace CPU time was unexpectedly low: {busy:.2f}s")
        if idle > 0.5:
            raise AssertionError(f"idle trace was charged for other work: {idle:.2f}s")
        return f"busy_cpu={busy:.2f}s,idle_cpu={idle:.2f}s"

    def _run_restart_case(self) -> str:
        process = self._launch_workload("restart", child_seconds=4.0, parent_seconds=3.5)
        self._wait_until(
            lambda: (self._inputs.work_dir / "child-cgroup-restart").is_file(),
            "restart workload did not start",
        )
        self._wait_until(lambda: self._event_count(4) > 0, "initial sample was not stored")
        before = self._event_count(4)
        database = self._config.parent / "actrail-resources.sqlite"
        before_profile = profile_snapshot(database, 4)
        before_samples = self._resource_samples(4)
        # A scope left in a single hierarchy (a crash during creation) is still an orphan.
        memory = self._v1_mounts()["memory"]
        orphan = memory / self._root_name / "traces/trace-999-00000000000000000000000000000000"
        (orphan / "workload").mkdir(parents=True)
        self._restart_daemon()
        self._wait_until(lambda: self._event_count(4) > before, "sampling did not resume")
        self._wait_until(lambda: not orphan.exists(), "empty startup orphan was not removed")
        process.communicate(timeout=8)
        self._wait_for_trace(4, "finalized")
        samples = self._resource_samples(4)
        payloads = [sample["payload"] for sample in samples]
        self._assert_launch_membership("restart")
        self._assert_final(payloads, coverage="exact")
        storage = assert_restart_storage(database, 4, before_profile, before_samples, samples)
        return f"events={len(payloads)}; {storage}"

    def _run_timeout_case(self) -> str:
        process = self._launch_workload("timeout", child_seconds=3.0, parent_seconds=0.2)
        process.communicate(timeout=8)
        child_pid = int(
            (self._inputs.work_dir / "child-pid-timeout").read_text(encoding="utf-8")
        )
        self._wait_for_trace(5, "orphaned")
        if not Path(f"/proc/{child_pid}").exists():
            raise AssertionError("finalization timeout killed the descendant")
        payloads = self._resource_payloads(5)
        self._assert_final(payloads, coverage="partial", timed_out=True)
        self._wait_until(
            lambda: (self._inputs.work_dir / "child-done-timeout").is_file(),
            "timeout descendant did not finish naturally",
            timeout=6,
        )
        self._wait_until(
            lambda: not any(path.exists() for path in self._scope_replicas(5)),
            "empty timed-out scope was not cleaned",
        )
        return f"events={len(payloads)},descendant={child_pid}"

    def _run_lost_scope_case(self) -> str:
        process = self._launch_workload("lost", child_seconds=1.0, parent_seconds=0.8)
        self._wait_until(lambda: self._event_count(6) > 0, "lost-scope workload did not sample")
        assert self._daemon is not None
        os.kill(self._daemon.pid, signal.SIGSTOP)
        try:
            self._wait_until(
                lambda: (self._inputs.work_dir / "child-done-lost").exists(),
                "lost-scope workload did not write its completion marker",
            )
            self._remove_lost_scopes(self._scope_replicas(6))
        finally:
            self._restart_daemon()
        process.communicate(timeout=8)
        self._wait_for_trace(6, "orphaned")
        payloads = self._resource_payloads(6)
        # A lost scope is closed by a diagnostic, not by a final sample.
        if any(payload["sample_kind"] == "final" for payload in payloads):
            raise AssertionError("lost scope unexpectedly produced a final sample")
        with sqlite3.connect(self._database) as connection:
            health = connection.execute(
                "SELECT health FROM traces WHERE trace_id=6"
            ).fetchone()
        if health != ("degraded",):
            raise AssertionError(f"lost scope did not degrade the trace: {health}")
        self._launch_workload("after-lost", child_seconds=0.5, parent_seconds=0.2).communicate(
            timeout=8
        )
        self._wait_for_trace(7, "finalized")
        self._assert_final(self._resource_payloads(7), coverage="exact")
        return f"events={len(payloads)},post_recovery_trace=7"

    def _remove_lost_scopes(self, scopes: list[Path]) -> None:
        """Remove only this run's scopes after tasks and controller references drain."""
        pending = [directory for scope in scopes for directory in (scope / "workload", scope)]
        last_busy: OSError | None = None

        def remove() -> bool:
            nonlocal last_busy
            while pending:
                directory = pending[0]
                try:
                    # The child writes its done marker before os._exit(); the marker alone
                    # does not establish that the kernel has detached it from every hierarchy.
                    if (directory / "cgroup.procs").read_text().strip():
                        return False
                    directory.rmdir()
                except FileNotFoundError:
                    if directory.exists():
                        raise
                except OSError as error:
                    if error.errno != errno.EBUSY:
                        raise
                    last_busy = error
                    return False
                pending.pop(0)
            return True

        try:
            self._wait_until(
                remove, "lost-scope cgroups did not become removable",
                timeout=CASE_CONSTANTS.run_lost_scope_case_timeout,
            )
        except TimeoutError as error:
            states = []
            for directory in pending:
                details = []
                for name in ("cgroup.procs", "tasks"):
                    try:
                        details.append(f"{name}={(directory / name).read_text().strip()!r}")
                    except OSError as read_error:
                        details.append(f"{name}=<{read_error}>")
                try:
                    children = [entry.name for entry in directory.iterdir() if entry.is_dir()]
                except OSError as read_error:
                    children = f"<{read_error}>"
                states.append(f"{directory}: {', '.join(details)}, children={children}")
            raise TimeoutError(
                f"lost-scope cgroups did not become removable; last EBUSY={last_busy}; " + "; ".join(states)
            ) from error

    def _launch_workload(
        self, suffix: str, child_seconds: float, parent_seconds: float
    ) -> subprocess.Popen[str]:
        return self._launch(
            suffix,
            (
                str(WORKLOAD),
                str(self._inputs.work_dir),
                suffix,
                str(child_seconds),
                str(parent_seconds),
            ),
        )

    def _launch_code(self, suffix: str, code: str) -> subprocess.Popen[str]:
        return self._launch(suffix, ("-c", code))

    def _launch(self, suffix: str, arguments: tuple[str, ...]) -> subprocess.Popen[str]:
        return subprocess.Popen(
            (
                str(self._binary("actrailctl")),
                "launch",
                "--config",
                str(self._config),
                "--name",
                f"resource-cgroup-v1-{suffix}",
                "--host-ebpf",
                "disabled",
                "--seccomp-notify",
                "disabled",
                "--",
                shutil.which("python3") or "python3",
                *arguments,
            ),
            cwd=self._inputs.repo,
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def _assert_launch_membership(self, suffix: str) -> None:
        def scoped(path: Path) -> list[str]:
            return [
                line.split(":", 2)[2]
                for line in path.read_text(encoding="utf-8").splitlines()
                if set(line.split(":", 2)[1].split(",")) & {"cpuacct", "memory"}
            ]

        parent = scoped(self._inputs.work_dir / f"parent-cgroup-{suffix}")
        child = scoped(self._inputs.work_dir / f"child-cgroup-{suffix}")
        prefix = f"/{self._root_name}/traces/trace-"
        if (
            parent != child
            or len(parent) != 2
            or not all(path.startswith(prefix) and path.endswith("/workload") for path in parent)
        ):
            raise AssertionError(
                f"launch was not admitted before exec: parent={parent!r}, child={child!r}"
            )

    @staticmethod
    def _assert_final(
        payloads: list[dict[str, object]],
        *,
        coverage: str,
        timed_out: bool = False,
    ) -> None:
        finals = [payload for payload in payloads if payload["sample_kind"] == "final"]
        if len(finals) != 1:
            raise AssertionError(f"expected exactly one final sample, got {len(finals)}")
        final = finals[0]
        if final["accounting_method"] != "cgroup_v1":
            raise AssertionError(f"unexpected accounting method: {final!r}")
        if final["accounting_coverage"] != coverage:
            raise AssertionError(f"unexpected accounting coverage: {final!r}")
        if timed_out and final["metadata"].get("finalization_timeout") != "true":
            raise AssertionError(f"timeout marker is absent: {final!r}")

    @staticmethod
    def _final_cpu_seconds(payloads: list[dict[str, object]]) -> float:
        finals = [payload for payload in payloads if payload["sample_kind"] == "final"]
        if len(finals) != 1 or finals[0]["cpu_usage_usec"] is None:
            raise AssertionError(f"no final CPU counter: {finals!r}")
        return finals[0]["cpu_usage_usec"] / 1_000_000

    def _wait_for_trace(self, trace_id: int, scope_state: str) -> None:
        self._wait_until(
            lambda: self._trace_state(trace_id) == ("completed", scope_state),
            f"trace-{trace_id} did not reach completed/{scope_state}",
            timeout=10,
        )

    def _trace_id(self, suffix: str) -> int:
        with sqlite3.connect(self._database) as connection:
            row = connection.execute(
                "SELECT trace_id FROM traces WHERE display_name = ?",
                (f"resource-cgroup-v1-{suffix}",),
            ).fetchone()
        if row is None:
            raise AssertionError(f"trace {suffix!r} is missing")
        return int(row[0])

    def _trace_state(self, trace_id: int) -> tuple[str, str] | None:
        if not self._database.is_file():
            return None
        with sqlite3.connect(self._database) as connection:
            row = connection.execute(
                "SELECT traces.lifecycle_state, trace_resource_scopes.lifecycle_state "
                "FROM traces JOIN trace_resource_scopes USING(trace_id) "
                "WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
        return None if row is None else (str(row[0]), str(row[1]))

    def _scope_replicas(self, trace_id: int) -> list[Path]:
        with sqlite3.connect(self._database) as connection:
            row = connection.execute(
                "SELECT relative_path, accounting_method FROM trace_resource_scopes "
                "WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
        if row is None or row[1] != "cgroup_v1":
            raise AssertionError(f"trace-{trace_id} has no managed v1 scope: {row}")
        return [mount / self._root_name / str(row[0]) for mount in self._hierarchies]

    def _event_count(self, trace_id: int) -> int:
        database = self._config.parent / "actrail-resources.sqlite"
        if not database.is_file():
            return 0
        with sqlite3.connect(database) as connection:
            row = connection.execute(
                "SELECT COUNT(*) FROM resource_workload_samples WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
        return int(row[0]) if row is not None else 0

    def _resource_payloads(self, trace_id: int) -> list[dict[str, object]]:
        return [sample["payload"] for sample in self._resource_samples(trace_id)]

    def _resource_samples(self, trace_id: int) -> list[dict[str, object]]:
        result = self._command(
            (
                str(self._binary("actrailviewer")),
                "--config", str(self._config),
                "--output-format", "json",
                "workload-resources", "--trace-id", f"trace-{trace_id}",
                "--limit", "10000",
            )
        )
        return json.loads(result.stdout)

    def _doctor_ok(self) -> bool:
        if self._daemon is not None and self._daemon.poll() is not None:
            raise RuntimeError(f"daemon exited with {self._daemon.returncode}")
        result = subprocess.run(
            (str(self._binary("actrailctl")), "doctor", "--config", str(self._config)),
            cwd=self._inputs.repo,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return result.returncode == 0

    def _remove_roots(self) -> None:
        """rmdir this run's managed roots bottom-up; never unlink control files."""
        for mount in self._hierarchies:
            root = mount / self._root_name
            if not root.is_dir():
                continue
            for directory, _, _ in sorted(os.walk(root), key=lambda e: -len(e[0])):
                try:
                    os.rmdir(directory)
                except OSError:
                    pass

    def _binary(self, name: str) -> Path:
        directory = self._inputs.bin_dir
        if not directory.is_absolute():
            directory = self._inputs.repo / directory
        return directory / name

    def _command(self, command: tuple[str, ...]) -> subprocess.CompletedProcess[str]:
        return subprocess.run(
            command,
            cwd=self._inputs.repo,
            check=True,
            capture_output=True,
            text=True,
            timeout=30,
        )

    @staticmethod
    def _wait_until(
        condition: Callable[[], bool],
        message: str,
        *,
        timeout: float = 8,
    ) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if condition():
                return
            time.sleep(0.05)
        raise TimeoutError(message)
