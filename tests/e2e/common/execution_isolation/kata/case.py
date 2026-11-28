"""Resolve isolation prerequisites and run the shared Kata scenario."""
from __future__ import annotations

from tests.e2e.common.kata_runtime.process import SubprocessRunner
from tests.e2e.common.core import TestRunDirectories, TestCase, TestResult, TestStatus
from tests.e2e.common.runner import TestingContextSingleton

from .config import KataExecutionIsolationConfig
from .prerequisites import KataExecutionIsolationPrerequisites
from .scenario.runtime import KataExecutionIsolationScenario


class KataExecutionIsolationCase(TestCase):
    config_type: type[KataExecutionIsolationConfig]

    def __init__(self, directories: TestRunDirectories) -> None:
        self._config = self.config_type.from_environment(directories)
        self._validator = (
            directories.repo
            / "tests/e2e/regression/virtual_container/validate-runtime-config.py"
        )

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case(f"{self._config.IDENTITY.DISPLAY} observation path") as case:
            with case.step("Check Kata prerequisites") as step:
                readiness = KataExecutionIsolationPrerequisites(self._config).resolve()
                if readiness.problem is not None:
                    if readiness.problem.effective_status() == TestStatus.SKIPPED:
                        case.skip(readiness.problem.message)
                    step.details["readiness"] = readiness.problem
                    return case.result
                assert readiness.deployment is not None
            with case.step("Validate the Kata deployment"):
                validation = SubprocessRunner().run(
                    (str(self._validator), "--backend", self._config.BACKEND,
                     "--require-kernel-config", "--require-ebpf",
                     str(readiness.deployment.data_config)),
                    timeout=self._config.command_timeout_seconds,
                )
                context.output.command_output(validation.stdout, validation.stderr)
                if validation.returncode != 0:
                    raise AssertionError(f"{self._config.IDENTITY.DISPLAY} data config is invalid")
            KataExecutionIsolationScenario(self._config, readiness.deployment).run(case)
        return case.result
