from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
from pathlib import Path

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import ContainerAgentXiaooConfig


class ContainerAgentXiaooCase(TestCase):
    def __init__(self, config: ContainerAgentXiaooConfig):
        self._config = config
        self._script = (
            config.repo
            / "tests/v2/regression/container_agent_xiaoo/v2/nested_observer_scenario.py"
        )
        self._inner_script = (
            config.repo
            / "tests/v2/regression/container_agent_xiaoo/v2/xiaoo_scenario.py"
        )

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        test_context.report_progress(
            "prerequisites",
            "checking Docker, release binaries, and xiaoO",
        )
        artifact_error = self._artifact_error()
        if artifact_error is not None:
            return TestResult(TestStatus.FAILED, artifact_error)

        prerequisite_error = self._external_prerequisite_error()
        if prerequisite_error is not None:
            return TestResult(TestStatus.SKIPPED, prerequisite_error)

        assert self._config.xiaoo_bin is not None
        command = [
            sys.executable,
            str(self._script),
            "--bin-dir",
            str(self._config.bin_dir.resolve()),
            "--image",
            self._config.image,
            "--xiaoo-bin",
            str(self._config.xiaoo_bin.resolve()),
            "--keep-runtime-on-failure",
        ]
        if self._config.rebuild_image:
            command.append("--rebuild-image")
        test_context.report_progress(
            "container_runtime",
            "running two long-lived containers and real xiaoO agents",
        )
        process = subprocess.Popen(
            command,
            cwd=self._config.repo,
            env=os.environ.copy(),
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )
        try:
            stdout, stderr = process.communicate(
                timeout=self._config.timeout_seconds
            )
        except subprocess.TimeoutExpired:
            stdout, stderr = self._terminate_process_group(process)
            test_context.output.command_output(stdout, stderr)
            return TestResult(
                TestStatus.FAILED,
                f"timed out after {self._config.timeout_seconds}s",
            )

        test_context.output.command_output(stdout, stderr)
        if process.returncode != 0:
            return TestResult(
                TestStatus.FAILED,
                "real xiaoO container E2E exited with status "
                f"{process.returncode}:\n{self._failure_report(stderr)}",
            )
        test_context.report_progress(
            "trace_validation",
            "container attribution and trace evidence validated",
        )
        return TestResult(
            TestStatus.PASSED,
            "real xiaoO multi-container acceptance completed",
        )

    def _artifact_error(self) -> str | None:
        required = (
            self._config.bin_dir / "actraild",
            self._config.bin_dir / "actrailctl",
            self._config.bin_dir / "actrailviewer",
            self._config.bin_dir / "libactrail_tls_payload_probe_sync.so",
            self._script,
            self._inner_script,
        )
        missing = [str(path) for path in required if not path.is_file()]
        if missing:
            return "required artifact(s) missing: " + ", ".join(missing)
        return None

    def _failure_report(self, stderr: str) -> str:
        lines = stderr.rstrip().splitlines()
        diagnostic_lines = lines[-80:]
        diagnostic = "\n".join(diagnostic_lines)
        if len(diagnostic) > 16_000:
            diagnostic = diagnostic[-16_000:]
        return diagnostic or "scenario produced no stderr diagnostics"

    def _external_prerequisite_error(self) -> str | None:
        missing_commands = [
            command
            for command in ("docker", "dockerd", "unshare", "bpftool", "ip")
            if shutil.which(command) is None
        ]
        if missing_commands:
            return (
                "external nested-container prerequisite is unavailable: "
                + ", ".join(missing_commands)
            )
        if self._config.xiaoo_bin is None:
            return (
                "real xiaoO executable is unavailable; set "
                "CONTAINER_AGENT_XIAOO_BINARY"
            )
        if not self._config.xiaoo_bin.is_file() or not os.access(
            self._config.xiaoo_bin,
            os.X_OK,
        ):
            return (
                "real xiaoO executable is unavailable: "
                f"{self._config.xiaoo_bin}"
            )
        try:
            completed = subprocess.run(
                ["docker", "info", "--format", "{{.ServerVersion}}"],
                text=True,
                capture_output=True,
                timeout=15,
                check=False,
            )
        except subprocess.TimeoutExpired:
            return "external container prerequisite is unavailable: Docker daemon timed out"
        if completed.returncode != 0:
            diagnostic = (completed.stderr or completed.stdout).strip()
            suffix = f": {diagnostic}" if diagnostic else ""
            return "external container prerequisite is unavailable: Docker daemon" + suffix
        return None

    def _terminate_process_group(
        self,
        process: subprocess.Popen[str],
    ) -> tuple[str, str]:
        try:
            os.killpg(process.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            return process.communicate(
                timeout=self._config.cleanup_grace_seconds
            )
        except subprocess.TimeoutExpired:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            return process.communicate()
