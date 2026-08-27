from __future__ import annotations

import subprocess
import sys
import tempfile
import unittest
from collections.abc import Mapping, Sequence
from pathlib import Path

from tests.v2.common.execution_isolation.controlled_oom import (
    ControlledHostOom,
    ControlledHostOomResult,
    MonitoredRootMarker,
    memory_cgroup_problem,
)
from tests.v2.common.process import CommandResult


class _RecordingRunner:
    def __init__(self, result: CommandResult) -> None:
        self._result = result
        self.argv: tuple[str, ...] | None = None
        self.timeout: float | None = None
        self.cwd: Path | None = None
        self.environment: dict[str, str] | None = None

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
        input_text: str | None = None,
    ) -> CommandResult:
        if input_text is not None:
            raise AssertionError("controlled OOM must not send subprocess input")
        self.argv = tuple(argv)
        self.timeout = timeout
        self.cwd = cwd
        self.environment = None if environment is None else dict(environment)
        return self._result


class _UnsafeCgroupRunner(_RecordingRunner):
    def __init__(self, result: CommandResult, unsafe_path: Path) -> None:
        super().__init__(result)
        self._unsafe_path = unsafe_path

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
        input_text: str | None = None,
    ) -> CommandResult:
        assert environment is not None
        coord_dir = Path(environment["ACTRAIL_HOST_COORD_DIR"])
        (coord_dir / "oom.cgroup").write_text(
            str(self._unsafe_path),
            encoding="ascii",
        )
        return super().run(
            argv,
            timeout=timeout,
            cwd=cwd,
            environment=environment,
            input_text=input_text,
        )


class ControlledHostOomTests(unittest.TestCase):
    def test_oom_asset_reaps_the_victim_before_removing_its_cgroup(self) -> None:
        script = (
            Path(__file__).resolve().parent
            / "assets"
            / "oom-cgroup-trigger.sh"
        ).read_text(encoding="utf-8")
        cleanup = script.split("cleanup() {", 1)[1].split("}\ntrap cleanup", 1)[0]

        kill_offset = cleanup.index('kill -KILL "$trigger_pid"')
        wait_offset = cleanup.index('wait "$trigger_pid" 2>/dev/null || true')
        clear_offset = cleanup.index('trigger_pid=""')
        rmdir_offset = cleanup.index('rmdir "$group"')
        self.assertLess(kill_offset, wait_offset)
        self.assertLess(wait_offset, clear_offset)
        self.assertLess(clear_offset, rmdir_offset)

    def test_controlled_oom_is_available_from_the_common_public_module(self) -> None:
        from tests.v2.common import execution_isolation

        self.assertIs(execution_isolation.ControlledHostOom, ControlledHostOom)
        self.assertIs(
            execution_isolation.ControlledHostOomResult,
            ControlledHostOomResult,
        )
        self.assertIs(
            execution_isolation.MonitoredRootMarker,
            MonitoredRootMarker,
        )
        self.assertIs(
            execution_isolation.memory_cgroup_problem,
            memory_cgroup_problem,
        )

    def test_module_command_rejects_missing_root_mode_arguments(self) -> None:
        completed = subprocess.run(
            (
                sys.executable,
                "-m",
                "tests.v2.common.execution_isolation.controlled_oom",
                "_run_monitored_root",
            ),
            cwd=Path(__file__).resolve().parents[4],
            capture_output=True,
            text=True,
            check=False,
        )

        self.assertNotEqual(completed.returncode, 0)
        self.assertIn("ROOT_DISCOVERY_SETTLE_SECONDS", completed.stderr)

    def test_memory_cgroup_problem_reports_an_unavailable_controller(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            self.assertEqual(
                memory_cgroup_problem(Path(directory)),
                "memory cgroup controller is unavailable",
            )

    def test_memory_cgroup_problem_requires_a_delegated_v2_memory_file(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cgroup_root = root / "cgroup"
            cgroup_root.mkdir()
            (cgroup_root / "cgroup.controllers").write_text(
                "memory\n",
                encoding="ascii",
            )
            swaps = root / "swaps"
            swaps.write_text(
                "Filename Type Size Used Priority\n",
                encoding="ascii",
            )

            self.assertEqual(
                memory_cgroup_problem(cgroup_root, swaps_path=swaps),
                f"memory controller is not enabled below {cgroup_root}",
            )
            self.assertEqual(
                list(cgroup_root.iterdir()),
                [cgroup_root / "cgroup.controllers"],
            )

    def test_active_host_swap_requires_a_cgroup_swap_limit(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            cgroup_root = root / "cgroup"
            cgroup_root.mkdir()
            (cgroup_root / "cgroup.controllers").write_text(
                "memory\n",
                encoding="ascii",
            )
            swaps = root / "swaps"
            swaps.write_text(
                "Filename Type Size Used Priority\n"
                "/dev/zram0 partition 1048572 0 100\n",
                encoding="ascii",
            )

            self.assertEqual(
                memory_cgroup_problem(cgroup_root, swaps_path=swaps),
                "active host swap cannot be bounded by this memory cgroup",
            )

    def test_run_monitored_returns_kernel_and_cgroup_evidence_with_deadline(
        self,
    ) -> None:
        output = (
            "ACTRAIL_CONTROLLED_HOST_OOM_ROOT "
            "pid=4321 start_time_ticks=987654 "
            "executable_name_hex=6163747261696c2d726f6f7400000000\n"
            "ACTRAIL_HOST_OOM_KILL_OK "
            "pid=5432 before=10 after=11 cgroup_before=0 cgroup_after=1 "
            "released_at_ms=1777000123456\n"
        )
        command_result = CommandResult(("fixture",), 0, output, "")
        runner = _RecordingRunner(command_result)

        with tempfile.TemporaryDirectory() as directory:
            work_dir = Path(directory)
            result = ControlledHostOom(work_dir, runner=runner).run_monitored(
                root_discovery_settle_seconds=0.25,
                timeout_seconds=7,
            )

            self.assertEqual(result.victim_pid, 5432)
            self.assertEqual(result.released_at_ms, 1777000123456)
            self.assertEqual(result.kernel_oom_kills_before, 10)
            self.assertEqual(result.kernel_oom_kills_after, 11)
            self.assertEqual(result.cgroup_oom_kills_before, 0)
            self.assertEqual(result.cgroup_oom_kills_after, 1)
            self.assertEqual(
                result.root_marker.as_process(),
                {
                    "pid": 4321,
                    "start_time_ticks": 987654,
                    "executable_name_hex": "6163747261696c2d726f6f7400000000",
                },
            )

        self.assertEqual(runner.timeout, 7)
        self.assertEqual(
            runner.argv[:4] if runner.argv is not None else None,
            (
                sys.executable,
                "-m",
                "tests.v2.common.execution_isolation.controlled_oom",
                "_run_monitored_root",
            ),
        )
        self.assertEqual(runner.cwd, Path(__file__).resolve().parents[4])
        self.assertIsNotNone(runner.environment)
        environment = runner.environment or {}
        oom_script = Path(environment["ACTRAIL_HOST_OOM_SCRIPT"])
        oom_trigger = Path(environment["ACTRAIL_HOST_OOM_TRIGGER"])
        coord_dir = Path(environment["ACTRAIL_HOST_COORD_DIR"])
        self.assertTrue(oom_script.is_file())
        self.assertTrue(oom_trigger.is_file())
        self.assertEqual(
            oom_script.parent,
            Path(__file__).resolve().parent / "assets",
        )
        self.assertEqual(oom_trigger.parent, oom_script.parent)
        self.assertEqual(coord_dir.parent, work_dir.resolve())
        self.assertTrue(coord_dir.name.startswith("controlled-host-oom-"))

    def test_run_monitored_rejects_missing_cgroup_oom_increment(self) -> None:
        output = (
            "ACTRAIL_CONTROLLED_HOST_OOM_ROOT "
            "pid=4321 start_time_ticks=987654 "
            "executable_name_hex=6163747261696c2d726f6f7400000000\n"
            "ACTRAIL_HOST_OOM_KILL_OK "
            "pid=5432 before=10 after=11 cgroup_before=1 cgroup_after=1 "
            "released_at_ms=1777000123456\n"
        )
        runner = _RecordingRunner(CommandResult(("fixture",), 0, output, ""))

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(
                RuntimeError,
                "cgroup OOM kill did not increase",
            ):
                ControlledHostOom(Path(directory), runner=runner).run_monitored(
                    root_discovery_settle_seconds=0.25,
                    timeout_seconds=7,
                )

    def test_run_monitored_never_removes_an_unsafe_recorded_path(self) -> None:
        output = (
            "ACTRAIL_CONTROLLED_HOST_OOM_ROOT "
            "pid=4321 start_time_ticks=987654 "
            "executable_name_hex=6163747261696c2d726f6f7400000000\n"
            "ACTRAIL_HOST_OOM_KILL_OK "
            "pid=5432 before=10 after=11 cgroup_before=0 cgroup_after=1 "
            "released_at_ms=1777000123456\n"
        )
        result = CommandResult(("fixture",), 0, output, "")
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            unsafe_path = root / "actrail-host-oom-unsafe"
            unsafe_path.mkdir()
            runner = _UnsafeCgroupRunner(result, unsafe_path)

            with self.assertRaisesRegex(RuntimeError, "unsafe cgroup path"):
                ControlledHostOom(root, runner=runner).run_monitored(
                    root_discovery_settle_seconds=0.25,
                    timeout_seconds=7,
                )

            self.assertTrue(unsafe_path.is_dir())

    def test_run_monitored_rejects_missing_kernel_oom_increment(self) -> None:
        output = (
            "ACTRAIL_CONTROLLED_HOST_OOM_ROOT "
            "pid=4321 start_time_ticks=987654 "
            "executable_name_hex=6163747261696c2d726f6f7400000000\n"
            "ACTRAIL_HOST_OOM_KILL_OK "
            "pid=5432 before=10 after=10 cgroup_before=0 cgroup_after=1 "
            "released_at_ms=1777000123456\n"
        )
        runner = _RecordingRunner(CommandResult(("fixture",), 0, output, ""))

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(
                RuntimeError,
                "kernel OOM kill did not increase",
            ):
                ControlledHostOom(Path(directory), runner=runner).run_monitored(
                    root_discovery_settle_seconds=0.25,
                    timeout_seconds=7,
                )

    def test_run_monitored_rejects_a_root_that_is_not_actrail_root(self) -> None:
        output = (
            "ACTRAIL_CONTROLLED_HOST_OOM_ROOT "
            "pid=4321 start_time_ticks=987654 "
            "executable_name_hex=707974686f6e33000000000000000000\n"
            "ACTRAIL_HOST_OOM_KILL_OK "
            "pid=5432 before=10 after=11 cgroup_before=0 cgroup_after=1 "
            "released_at_ms=1777000123456\n"
        )
        runner = _RecordingRunner(CommandResult(("fixture",), 0, output, ""))

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(
                RuntimeError,
                "root comm is not actrail-root",
            ):
                ControlledHostOom(Path(directory), runner=runner).run_monitored(
                    root_discovery_settle_seconds=0.25,
                    timeout_seconds=7,
                )

    def test_run_monitored_rejects_a_nonpositive_victim_pid(self) -> None:
        output = (
            "ACTRAIL_CONTROLLED_HOST_OOM_ROOT "
            "pid=4321 start_time_ticks=987654 "
            "executable_name_hex=6163747261696c2d726f6f7400000000\n"
            "ACTRAIL_HOST_OOM_KILL_OK "
            "pid=0 before=10 after=11 cgroup_before=0 cgroup_after=1 "
            "released_at_ms=1777000123456\n"
        )
        runner = _RecordingRunner(CommandResult(("fixture",), 0, output, ""))

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(RuntimeError, "victim PID is invalid"):
                ControlledHostOom(Path(directory), runner=runner).run_monitored(
                    root_discovery_settle_seconds=0.25,
                    timeout_seconds=7,
                )

    def test_run_monitored_requires_time_after_root_discovery(self) -> None:
        runner = _RecordingRunner(CommandResult(("fixture",), 0, "", ""))

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(
                ValueError,
                "timeout_seconds must exceed root discovery settle time",
            ):
                ControlledHostOom(Path(directory), runner=runner).run_monitored(
                    root_discovery_settle_seconds=1,
                    timeout_seconds=1,
                )

        self.assertIsNone(runner.argv)

    def test_run_monitored_requires_positive_root_discovery_settle(self) -> None:
        runner = _RecordingRunner(CommandResult(("fixture",), 0, "", ""))

        with tempfile.TemporaryDirectory() as directory:
            with self.assertRaisesRegex(
                ValueError,
                "root_discovery_settle_seconds must be positive",
            ):
                ControlledHostOom(Path(directory), runner=runner).run_monitored(
                    root_discovery_settle_seconds=0,
                    timeout_seconds=1,
                )

        self.assertIsNone(runner.argv)


if __name__ == "__main__":
    unittest.main()
