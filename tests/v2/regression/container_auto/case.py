from __future__ import annotations

import os
import shutil
import signal
import subprocess
from pathlib import Path

from tests.v2.common.test_case import TestCase, TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton

from .config import ContainerAutoConfig


class ContainerAutoCase(TestCase):
    def __init__(self, config: ContainerAutoConfig):
        self._config = config
        self._script = config.repo / "deploy/container-auto/e2e.sh"

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        artifact_error = self._artifact_error()
        if artifact_error is not None:
            return TestResult(TestStatus.FAILED, artifact_error)

        prerequisite_error = self._external_prerequisite_error()
        if prerequisite_error is not None:
            return TestResult(TestStatus.SKIPPED, prerequisite_error)

        environment = os.environ.copy()
        environment["BIN_DIR"] = str(self._config.bin_dir.resolve())
        command = ["/usr/bin/env", "bash", str(self._script)]
        test_context.output.line("+ " + " ".join(command))

        process = subprocess.Popen(
            command,
            cwd=self._config.repo,
            env=environment,
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
                f"container deployment E2E exited with status {process.returncode}",
            )
        return TestResult(
            TestStatus.PASSED,
            "ordinary container deployment acceptance completed",
        )

    def _artifact_error(self) -> str | None:
        required = (
            self._config.bin_dir / "actraild",
            self._config.bin_dir / "actrailctl",
            self._config.bin_dir / "libactrail_tls_payload_probe_sync.so",
            self._script,
        )
        missing = [str(path) for path in required if not path.is_file()]
        if missing:
            return "required artifact(s) missing: " + ", ".join(missing)
        return None

    @staticmethod
    def _external_prerequisite_error() -> str | None:
        for command in ("bash", "docker", "sqlite3"):
            if shutil.which(command) is None:
                return f"external container prerequisite is unavailable: {command}"
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
