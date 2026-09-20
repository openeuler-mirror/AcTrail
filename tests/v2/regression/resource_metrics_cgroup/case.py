from __future__ import annotations

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


class ResourceMetricsCgroupCase(TestCase):
    """Release-binary acceptance for daemon-owned cgroup-v2 trace scopes."""

    def __init__(self, inputs: TestCaseInputs) -> None:
        self._inputs = inputs
        self._unit = f"actrail-resource-metrics-v2-{os.getpid()}"
        self._service = f"{self._unit}.service"
        self._config = inputs.work_dir / "actraild.conf"
        self._socket = inputs.work_dir / "run/control.sock"
        self._pid_file = inputs.work_dir / "run/actraild.pid"
        self._database = inputs.work_dir / "data/actrail.sqlite"
        self._cgroup = Path("/sys/fs/cgroup/system.slice") / self._service

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        del test_context
        problem = self._prerequisite_problem()
        if problem is not None:
            return TestResult(TestStatus.SKIPPED, problem)
        try:
            self._prepare_config()
            self._start_daemon()
            self._assert_hierarchy()
            exact = self._run_exact_case()
            restarted = self._run_restart_case()
            timed_out = self._run_timeout_case()
            lost = self._run_lost_scope_case()
            return TestResult(
                TestStatus.PASSED,
                "cgroup-v2 resource metrics passed: "
                f"exact={exact}, restart={restarted}, timeout={timed_out}, lost={lost}",
            )
        except Exception as error:
            return TestResult(TestStatus.FAILED, str(error))
        finally:
            self._stop_units()

    def cleanup(self, test_context: TestingContextSingleton) -> TestResult | None:
        del test_context
        self._stop_units()
        return None

    def _prerequisite_problem(self) -> str | None:
        if os.geteuid() != 0:
            return "resource_metrics_cgroup requires root for delegated cgroup setup"
        if not Path("/sys/fs/cgroup/cgroup.controllers").is_file():
            return "unified cgroup v2 is unavailable"
        if not Path("/run/systemd/system").is_dir():
            return "systemd is not the active service manager"
        missing = [
            name
            for name in ("systemctl", "systemd-run")
            if shutil.which(name) is None
        ]
        binaries = [
            self._binary("actraild"),
            self._binary("actrailctl"),
            self._binary("actrailviewer"),
        ]
        missing.extend(str(path) for path in binaries if not path.is_file())
        if missing:
            return (
                "missing required commands or release binaries: "
                + ", ".join(missing)
            )
        version = self._command(("systemd-run", "--version")).stdout.splitlines()[0]
        try:
            systemd_version = int(version.split()[1])
        except (IndexError, ValueError):
            return f"could not parse systemd version: {version!r}"
        if systemd_version < 254:
            return "systemd DelegateSubgroup support requires systemd 254 or newer"
        return None

    def _prepare_config(self) -> None:
        for directory in ("run", "data", "log", "export", "plugins"):
            (self._inputs.work_dir / directory).mkdir(parents=True, exist_ok=True)
        patch = self._inputs.work_dir / "operator.patch.toml"
        patch.write_text(
            self._operator_patch(),
            encoding="utf-8",
        )
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
pid_file = "{self._pid_file}"
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

[hand_observation]
enabled = false

[sandbox_alerts]
enabled = false

[capture]
profile_name = "resource-metrics-cgroup-v2"
capabilities = ["resource-metrics"]
opportunistic_capabilities = []
disabled_capabilities = []

[ebpf]
enabled = "false"

[payload.tls]
enabled = false
sync_event_socket_path = "{work / 'run/tls-sync.sock'}"

[payload.stdio]
enabled = false

[payload.socket]
enabled = false

[seccomp_notify]
enabled = false

[process_seccomp]
enabled = false

[agent_invocation]
enabled = false

[file_observation]
enabled = false

[application]
enabled = false
http1_enabled = false
http2_enabled = false

[resource_metrics]
enabled = true
mode = "cgroup-v2"
interval_ms = 50
cgroup_root = "{self._cgroup}"
finalization_timeout_ms = 1000
orphan_limit = 64
memory_alert_rss_kb = "1"

[enforcement]
enabled = false
'''

    def _start_daemon(self) -> None:
        self._command(
            (
                "systemd-run",
                f"--unit={self._unit}",
                "--property=Delegate=yes",
                "--property=DelegateSubgroup=daemon",
                "--property=Type=exec",
                "--property=KillMode=process",
                "--property=Restart=on-failure",
                "--property=RestartSec=200ms",
                f"--property=WorkingDirectory={self._inputs.repo}",
                str(self._binary("actraild")),
                "--config",
                str(self._config),
                "run",
            )
        )
        self._wait_until(
            lambda: self._service_pid() > 0 and self._doctor_ok(),
            "daemon did not become ready",
        )

    def _assert_hierarchy(self) -> None:
        daemon_pid = self._service_pid()
        membership = Path(f"/proc/{daemon_pid}/cgroup").read_text(encoding="utf-8")
        if not membership.rstrip().endswith(f"/{self._service}/daemon"):
            raise AssertionError(f"daemon is not in delegated leaf: {membership!r}")
        root_processes = (self._cgroup / "cgroup.procs").read_text(encoding="utf-8")
        traces_processes = (self._cgroup / "traces/cgroup.procs").read_text(
            encoding="utf-8"
        )
        if root_processes.strip() or traces_processes.strip():
            raise AssertionError("managed hierarchy contains an internal-node process")

    def _run_exact_case(self) -> str:
        process = self._launch_process("exact", child_seconds=0.8, parent_seconds=0.2)
        self._wait_until(lambda: (self._inputs.work_dir / "child-cgroup-exact").exists(),
                         "exact workload did not start")
        scope = self._trace_scope_path(1)
        (scope / "workload/nested/leaf").mkdir(parents=True)
        process.communicate(timeout=8)
        if process.returncode != 0:
            raise AssertionError("exact workload launch failed")
        self._wait_for_trace(1, "finalized")
        self._wait_until(lambda: not scope.exists(), "nested empty cgroup cleanup did not finish")
        payloads = self._resource_payloads(1)
        self._assert_launch_membership("exact")
        self._assert_final(payloads, coverage="exact")
        peak = max(payload["memory_peak_bytes"] or 0 for payload in payloads)
        if peak < 48 * 1024 * 1024:
            raise AssertionError(f"kernel memory peak was unexpectedly low: {peak}")
        if max(payload["pids_peak"] or 0 for payload in payloads) < 2:
            raise AssertionError("kernel pids.peak did not observe the descendant")
        alerted_rss = [
            payload
            for payload in payloads
            if (payload["process_rss_sum_kb"] or 0) >= 1
            and payload["metadata"].get("alert") == "true"
        ]
        if not alerted_rss:
            raise AssertionError("configured RSS alert did not fire for cgroup samples")
        return f"events={len(payloads)},peak={peak}"

    def _run_restart_case(self) -> str:
        process = self._launch_process("restart", child_seconds=4.0, parent_seconds=3.5)
        self._wait_until(
            lambda: (self._inputs.work_dir / "child-cgroup-restart").is_file(),
            "restart workload did not start",
        )
        self._wait_until(
            lambda: self._event_count(2) > 0,
            "initial sample was not stored",
        )
        before = self._event_count(2)
        # Simulate a crash after trace-1's atomic final-event/scope commit but
        # before its separate trace terminal-state write.
        with sqlite3.connect(self._database) as connection:
            connection.execute(
                "UPDATE traces SET lifecycle_state = 'draining', completed_at = NULL "
                "WHERE trace_id = 1"
            )
        orphan = self._cgroup / "traces/trace-999-00000000000000000000000000000000"
        (orphan / "workload").mkdir(parents=True)
        previous_pid = self._service_pid()
        os.kill(previous_pid, signal.SIGKILL)
        self._wait_until(
            lambda: self._service_pid() not in (0, previous_pid) and self._doctor_ok(),
            "daemon did not restart after SIGKILL",
        )
        self._wait_until(
            lambda: self._event_count(2) > before,
            "sampling did not resume",
        )
        self._wait_until(
            lambda: self._trace_state(1) == ("completed", "finalized"),
            "persisted final barrier did not repair the trace state",
        )
        self._wait_until(
            lambda: not orphan.exists(),
            "empty startup orphan was not removed",
        )
        process.communicate(timeout=8)
        self._wait_for_trace(2, "finalized")
        payloads = self._resource_payloads(2)
        self._assert_launch_membership("restart")
        self._assert_final(payloads, coverage="exact")
        return f"events={len(payloads)},restarted_pid={self._service_pid()}"

    def _run_timeout_case(self) -> str:
        self._launch("timeout", child_seconds=3.0, parent_seconds=0.2)
        child_pid = int(
            (self._inputs.work_dir / "child-pid-timeout").read_text(encoding="utf-8")
        )
        self._wait_for_trace(3, "orphaned")
        if not Path(f"/proc/{child_pid}").exists():
            raise AssertionError("finalization timeout killed the descendant")
        payloads = self._resource_payloads(3)
        self._assert_final(payloads, coverage="partial", timed_out=True)
        self._wait_until(
            lambda: (self._inputs.work_dir / "child-done-timeout").is_file(),
            "timeout descendant did not finish naturally",
            timeout=6,
        )
        self._wait_until(
            lambda: not self._trace_scope_path(3).exists(),
            "empty timed-out scope was not cleaned",
        )
        return f"events={len(payloads)},descendant={child_pid}"

    def _run_lost_scope_case(self) -> str:
        process = self._launch_process("lost", child_seconds=1.0, parent_seconds=0.8)
        self._wait_until(lambda: self._event_count(4) > 0, "lost-scope workload did not sample")
        old_pid = self._service_pid()
        os.kill(old_pid, signal.SIGSTOP)
        try:
            self._wait_until(lambda: (self._inputs.work_dir / "child-done-lost").exists(),
                             "lost-scope workload did not exit")
            scope = self._trace_scope_path(4)
            # This is the exact disposable scope created by this acceptance run.
            # Remove it while the daemon is stopped to emulate external removal.
            (scope / "workload").rmdir()
            scope.rmdir()
        finally:
            os.kill(old_pid, signal.SIGKILL)
        self._wait_until(lambda: self._service_pid() not in (0, old_pid) and self._doctor_ok(),
                         "daemon did not recover from lost scope")
        process.communicate(timeout=8)
        self._wait_for_trace(4, "orphaned")
        payloads = self._resource_payloads(4)
        self._assert_final(payloads, coverage="partial")
        final = next(p for p in payloads if p["sample_kind"] == "final")
        if "recovery_scope_error" not in final["metadata"]:
            raise AssertionError("lost-scope final sample has no recovery diagnostic")
        with sqlite3.connect(self._database) as connection:
            health = connection.execute("SELECT health FROM traces WHERE trace_id=4").fetchone()
        if health != ("degraded",):
            raise AssertionError(f"lost scope did not degrade the trace: {health}")
        # New live events after startup must not reuse recovery event IDs.
        self._launch("after-lost", child_seconds=0.8, parent_seconds=0.2)
        self._wait_for_trace(5, "finalized")
        self._assert_final(self._resource_payloads(5), coverage="exact")
        return f"events={len(payloads)},post_recovery_trace=5"

    def _launch(self, suffix: str, child_seconds: float, parent_seconds: float) -> None:
        completed = self._launch_process(suffix, child_seconds, parent_seconds)
        completed.communicate(timeout=8)
        if completed.returncode != 0:
            raise RuntimeError(f"launch {suffix} failed rc={completed.returncode}")

    def _launch_process(
        self, suffix: str, child_seconds: float, parent_seconds: float
    ) -> subprocess.Popen[str]:
        return subprocess.Popen(
            (
                str(self._binary("actrailctl")),
                "launch",
                "--config",
                str(self._config),
                "--name",
                f"resource-cgroup-{suffix}",
                "--host-ebpf",
                "disabled",
                "--seccomp-notify",
                "disabled",
                "--",
                shutil.which("python3") or "python3",
                str(Path(__file__).with_name("workload.py")),
                str(self._inputs.work_dir),
                suffix,
                str(child_seconds),
                str(parent_seconds),
            ),
            cwd=self._inputs.repo,
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

    def _assert_launch_membership(self, suffix: str) -> None:
        parent = (self._inputs.work_dir / f"parent-cgroup-{suffix}").read_text(
            encoding="utf-8"
        )
        child = (self._inputs.work_dir / f"child-cgroup-{suffix}").read_text(
            encoding="utf-8"
        )
        if parent != child or "/traces/trace-" not in parent or not parent.endswith(
            "/workload"
        ):
            raise AssertionError(
                "launch was not admitted before exec: "
                f"parent={parent!r}, child={child!r}"
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
            raise AssertionError(
                f"expected exactly one final sample, got {len(finals)}"
            )
        final = finals[0]
        if final["accounting_method"] != "cgroup_v2":
            raise AssertionError(f"unexpected accounting method: {final!r}")
        if final["accounting_coverage"] != coverage:
            raise AssertionError(f"unexpected accounting coverage: {final!r}")
        metadata = final["metadata"]
        if timed_out and metadata.get("finalization_timeout") != "true":
            raise AssertionError(f"timeout marker is absent: {final!r}")

    def _wait_for_trace(self, trace_id: int, scope_state: str) -> None:
        self._wait_until(
            lambda: self._trace_state(trace_id) == ("completed", scope_state),
            f"trace-{trace_id} did not reach completed/{scope_state}",
            timeout=10,
        )

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

    def _trace_scope_path(self, trace_id: int) -> Path:
        with sqlite3.connect(self._database) as connection:
            row = connection.execute(
                "SELECT relative_path FROM trace_resource_scopes WHERE trace_id = ?",
                (trace_id,),
            ).fetchone()
        if row is None:
            raise AssertionError(f"trace-{trace_id} resource scope is missing")
        return self._cgroup / str(row[0])

    def _event_count(self, trace_id: int) -> int:
        if not self._database.is_file():
            return 0
        with sqlite3.connect(self._database) as connection:
            row = connection.execute(
                "SELECT COUNT(*) FROM events WHERE trace_id = ? AND kind = 'resource'",
                (trace_id,),
            ).fetchone()
        return int(row[0]) if row is not None else 0

    def _resource_payloads(self, trace_id: int) -> list[dict[str, object]]:
        result = self._command(
            (
                str(self._binary("actrailviewer")),
                "--storage-path",
                str(self._database),
                "--output-format",
                "json",
                "events",
                "--trace-id",
                f"trace-{trace_id}",
            )
        )
        events = json.loads(result.stdout)["events"]
        return [event["payload"] for event in events if event["variant"] == "resource"]

    def _doctor_ok(self) -> bool:
        result = subprocess.run(
            (
                str(self._binary("actrailctl")),
                "doctor",
                "--config",
                str(self._config),
            ),
            cwd=self._inputs.repo,
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        return result.returncode == 0

    def _service_pid(self) -> int:
        result = subprocess.run(
            ("systemctl", "show", "--property=MainPID", "--value", self._service),
            check=False,
            capture_output=True,
            text=True,
        )
        try:
            return int(result.stdout.strip())
        except ValueError:
            return 0

    def _stop_units(self) -> None:
        subprocess.run(
            ("systemctl", "stop", self._service),
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )
        subprocess.run(
            ("systemctl", "reset-failed", self._service),
            check=False,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
        )

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
