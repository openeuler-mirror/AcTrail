from __future__ import annotations

import shutil
import subprocess

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import ContainerAutoConfig
from .auto_scenario import ContainerAutoScenario


class ContainerAutoCase(TestCase):
    def __init__(self, config: ContainerAutoConfig):
        self._config = config
        self._scenario_path = config.repo / (
            "tests/v2/regression/container_auto/v2/auto_scenario.py"
        )

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        problem = self._prerequisite_problem()
        if problem is not None:
            return problem
        test_context.report_progress(
            "prerequisites",
            "checking Docker and release binaries",
        )
        ContainerAutoScenario(self._config, test_context).run()
        return TestResult(
            TestStatus.PASSED,
            "container permission auto-selection matrix completed",
        )

    def _prerequisite_problem(self) -> TestResult | None:
        required = (
            self._config.bin_dir / "actraild",
            self._config.bin_dir / "actrailctl",
            self._config.bin_dir / "actrailviewer",
            self._config.bin_dir / "libactrail_tls_payload_probe_sync.so",
            self._scenario_path,
            self._config.repo
            / "tests/v2/regression/container_auto/v2/Dockerfile",
            self._config.repo
            / "tests/v2/regression/container_auto/v2/operator.conf",
            self._config.repo
            / "tests/v2/regression/container_auto/v2/seccomp/actrail-notify.json",
        )
        missing = [str(path) for path in required if not path.is_file()]
        if missing:
            return TestResult(
                TestStatus.FAILED,
                "required artifact(s) missing: " + ", ".join(missing),
            )
        if shutil.which("docker") is None:
            return TestResult(TestStatus.SKIPPED, "docker is unavailable")
        completed = subprocess.run(
            ["docker", "info", "--format", "{{.ServerVersion}}"],
            text=True,
            capture_output=True,
            timeout=15,
            check=False,
        )
        if completed.returncode != 0:
            return TestResult(TestStatus.SKIPPED, "Docker daemon is unavailable")
        return None
