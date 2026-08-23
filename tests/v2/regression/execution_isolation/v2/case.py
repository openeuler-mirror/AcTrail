from __future__ import annotations

from tests.v2.common.kata_runtime.process import SubprocessRunner
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import ExecutionIsolationConfig
from .prerequisites import ExecutionIsolationPrerequisites
from .scenario import ExecutionIsolationScenario


class ExecutionIsolationCase(TestCase):
    def __init__(self, config: ExecutionIsolationConfig) -> None:
        self._config = config
        self._validator = (
            config.repo
            / "tests/v2/regression/virtual_container/validate-runtime-config.py"
        )

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        readiness = ExecutionIsolationPrerequisites(self._config).resolve()
        if readiness.problem is not None:
            return readiness.problem
        assert readiness.deployment is not None

        test_context.report_progress(
            "preflight",
            "validating refreshed Cloud Hypervisor data profile",
        )
        validation = SubprocessRunner().run(
            (
                str(self._validator),
                "--backend",
                "cloud-hypervisor",
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
                "execution-isolation Cloud Hypervisor data config is invalid",
            )
        return ExecutionIsolationScenario(
            self._config,
            test_context,
            readiness.deployment,
        ).run()
