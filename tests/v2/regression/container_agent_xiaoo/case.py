from __future__ import annotations

import os
import shutil
import signal
import subprocess
import sys
from pathlib import Path

from tests.v2.common.test_case import TestCase, TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton

from .config import ContainerAgentXiaooConfig


class ContainerAgentXiaooCase(TestCase):
    def __init__(self, config: ContainerAgentXiaooConfig):
        self._config = config
        self._script = (
            config.repo
            / "tests/agent-trace/multi-container-xiaoo/run_e2e.py"
        )

    def run(self, test_context: TestingContextSingleton) -> TestResult:
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
        ]
        test_context.output.line("+ " + " ".join(command))
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
                f"real xiaoO container E2E exited with status {process.returncode}",
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
        )
        missing = [str(path) for path in required if not path.is_file()]
        if missing:
            return "required artifact(s) missing: " + ", ".join(missing)
        return None

    def _external_prerequisite_error(self) -> str | None:
        if shutil.which("docker") is None:
            return "external container prerequisite is unavailable: docker"
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
