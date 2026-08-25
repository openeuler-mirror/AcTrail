from __future__ import annotations

from tests.v2.common.kata_runtime.process import SubprocessRunner
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import CloudHypervisorExecutionIsolationConfig
from .prerequisites import CloudHypervisorExecutionIsolationPrerequisites
from .scenario import CloudHypervisorExecutionIsolationScenario


class CloudHypervisorExecutionIsolationCase(TestCase):
    def __init__(self, config: CloudHypervisorExecutionIsolationConfig) -> None:
        self._config = config
        self._validator = (
            config.repo
            / "tests/v2/regression/virtual_container/validate-runtime-config.py"
        )

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        readiness = CloudHypervisorExecutionIsolationPrerequisites(
            self._config
        ).resolve()
        if readiness.problem is not None:
            return readiness.problem
        assert readiness.deployment is not None

        test_context.report_progress(
            "preflight",
            f"validating refreshed {self._config.IDENTITY.DISPLAY} data profile",
        )
        validation = SubprocessRunner().run(
            (
                str(self._validator),
                "--backend",
                self._config.BACKEND,
                "--require-kernel-config",
                "--require-ebpf",
                str(readiness.deployment.data_config),
            ),
            timeout=self._config.command_timeout_seconds,
        )
        test_context.output.command_output(validation.stdout, validation.stderr)
        if validation.returncode != 0:
            return TestResult(
                TestStatus.FAILED,
                f"{self._config.IDENTITY.DISPLAY} data config is invalid",
            )
        return CloudHypervisorExecutionIsolationScenario(
            self._config,
            test_context,
            readiness.deployment,
        ).run()
