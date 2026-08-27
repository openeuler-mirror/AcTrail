from __future__ import annotations

from pathlib import Path

from tests.v2.common.execution_isolation import SandboxAlertPath


class OomAlertAssertions:
    """Validate one test-owned OOM victim across stimulus and alert storage."""

    @staticmethod
    def read_victim_pid(path: Path) -> int:
        raw = path.read_text(encoding="ascii").strip()
        if not raw.isdigit() or int(raw) <= 0:
            raise RuntimeError(f"OOM victim PID is invalid: {raw!r}")
        return int(raw)

    @staticmethod
    def assert_single(
        alert_path: SandboxAlertPath,
        root_marker: tuple[int, int, str],
        child_release_ms: int,
        victim_pid: int,
    ) -> None:
        expected_process = {
            "pid": root_marker[0],
            "start_time_ticks": root_marker[1],
            "executable_name_hex": root_marker[2],
        }
        matches = [
            record
            for record in alert_path.database.records()
            if record.category == "sandbox.resource.oom_killed"
            and record.detected_at_ms >= child_release_ms
            and record.process == expected_process
            and record.extras.get("victim_pid") == victim_pid
        ]
        if len(matches) != 1:
            raise AssertionError(
                f"expected one alert for OOM victim {victim_pid}, found {len(matches)}"
            )
