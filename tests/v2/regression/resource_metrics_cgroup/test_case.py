from __future__ import annotations

import errno
import signal
import sqlite3
import tempfile
import tomllib
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import Mock, patch

from tests.v2.common.core import TestCaseInputs
from tests.v2.regression.resource_metrics_cgroup.case import (
    PATCH_CONFIG,
    ResourceMetricsCgroupCase,
)
from tests.v2.regression.resource_metrics_cgroup.cgroup_v1 import ResourceMetricsCgroupV1Case


class ResourceMetricsCgroupContractTest(unittest.TestCase):
    def test_patch_keeps_required_mode_and_delegated_service_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            inputs = TestCaseInputs(
                repo=Path("/repo"),
                bin_dir=Path("target/release"),
                work_dir=Path(directory),
            )
            case = ResourceMetricsCgroupCase(inputs)

            values = case._operator_values()
            patch = tomllib.loads(PATCH_CONFIG.read_text(encoding="utf-8"))
            workload = patch["resource_profiling"]["collection"]["workload"]
            managed = values["resource_profiling"]["collection"]["workload"]["cgroup"]["v2"]["managed"]

            self.assertEqual(workload["mode"], "cgroup")
            self.assertIn("/sys/fs/cgroup/system.slice/", managed["root"])
            self.assertEqual(workload["cgroup"]["v2"]["managed"]["finalization_timeout_ms"], 1000)


class CgroupV1ScopeDeletionTest(unittest.TestCase):
    def setUp(self) -> None:
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.work = Path(temporary.name)
        self.case = ResourceMetricsCgroupV1Case(TestCaseInputs(
            repo=self.work, bin_dir=self.work, work_dir=self.work,
        ))
        self.scope = self.work / "scope"
        self.leaf = self.scope / "workload"
        self.leaf.mkdir(parents=True)
        for directory in (self.scope, self.leaf):
            (directory / "cgroup.procs").write_text("")
            (directory / "tasks").write_text("")

    def test_lost_scope_waits_for_exit_and_retries_busy_leaf_and_parent(self) -> None:
        # The actual lost-scope call site sees the marker while the child is still attached.
        (self.work / "child-done-lost").touch()
        (self.leaf / "cgroup.procs").write_text("42\n")
        (self.leaf / "tasks").write_text("42\n")
        self.case._daemon = SimpleNamespace(pid=12345)
        self.case._launch_workload = Mock(return_value=Mock())
        self.case._event_count = Mock(return_value=1)
        self.case._scope_replicas = Mock(return_value=[self.scope])
        self.case._restart_daemon = Mock()
        self.case._wait_for_trace = Mock()
        self.case._resource_payloads = Mock(return_value=[])
        self.case._assert_final = Mock()
        self.case._database.parent.mkdir()
        with sqlite3.connect(self.case._database) as connection:
            connection.execute("CREATE TABLE traces(trace_id INTEGER, health TEXT)")
            connection.execute("INSERT INTO traces VALUES (6,'degraded')")

        attempted_while_populated = []
        attempts = {self.leaf: 0, self.scope: 0}
        rmdir = Path.rmdir

        def remove(directory: Path) -> None:
            if (directory / "cgroup.procs").read_text().strip():
                attempted_while_populated.append(directory)
                raise OSError(errno.EBUSY, "Device or resource busy", str(directory))
            attempts[directory] += 1
            # Even after tasks exit, a controller can temporarily retain a reference.
            if attempts[directory] == 1:
                raise OSError(errno.EBUSY, "Device or resource busy", str(directory))
            for entry in directory.iterdir():
                entry.unlink()
            rmdir(directory)

        def child_exits(_seconds: float) -> None:
            if self.leaf.exists():
                (self.leaf / "cgroup.procs").write_text("")
                (self.leaf / "tasks").write_text("")

        with patch("tests.v2.regression.resource_metrics_cgroup.cgroup_v1.os.kill") as stop, \
                patch("tests.v2.regression.resource_metrics_cgroup.cgroup_v1.time.sleep", side_effect=child_exits), \
                patch.object(Path, "rmdir", autospec=True, side_effect=remove):
            self.case._run_lost_scope_case()

        self.assertEqual(attempted_while_populated, [])
        self.assertEqual(attempts, {self.leaf: 2, self.scope: 2})
        self.assertFalse(self.scope.exists())
        stop.assert_called_once_with(12345, signal.SIGSTOP)
        self.case._restart_daemon.assert_called_once()

    def test_busy_timeout_reports_tasks_and_remaining_directories(self) -> None:
        busy = OSError(errno.EBUSY, "Device or resource busy", str(self.leaf))
        with patch.object(Path, "rmdir", autospec=True, side_effect=busy), \
                patch("tests.v2.regression.resource_metrics_cgroup.cgroup_v1.time.monotonic", side_effect=[0, 0, 9]), \
                patch("tests.v2.regression.resource_metrics_cgroup.cgroup_v1.time.sleep"):
            with self.assertRaises(TimeoutError) as caught:
                self.case._remove_lost_scopes([self.scope])
        message = str(caught.exception)
        self.assertIn(str(self.leaf), message)
        self.assertIn("Device or resource busy", message)
        self.assertIn("cgroup.procs=''", message)
        self.assertIn("tasks=''", message)
        self.assertIn("children=['workload']", message)

    def test_permission_error_is_not_retried(self) -> None:
        denied = PermissionError(errno.EACCES, "Permission denied", str(self.leaf))
        with patch.object(Path, "rmdir", autospec=True, side_effect=denied) as remove, \
                patch("tests.v2.regression.resource_metrics_cgroup.cgroup_v1.time.sleep") as sleep:
            with self.assertRaises(PermissionError):
                self.case._remove_lost_scopes([self.scope])
        remove.assert_called_once_with(self.leaf)
        sleep.assert_not_called()


if __name__ == "__main__":
    unittest.main()
