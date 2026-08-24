from __future__ import annotations

import os
from pathlib import Path

from tests.v2.common.agent_selection import AgentSelector
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import SandboxResourceAlertHostConfig
from .scenario import SandboxResourceAlertHostScenario


class SandboxResourceAlertHostCase(TestCase):
    def __init__(self, config: SandboxResourceAlertHostConfig) -> None:
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        problem = self._repository_problem()
        if problem is not None:
            return problem
        problem = self._host_problem()
        if problem is not None:
            return problem
        agent = AgentSelector(self._config.repo).select(
            test_context,
            kinds=("xiaoo",),
        )
        if agent is None:
            return TestResult(
                TestStatus.SKIPPED,
                "real xiaoO executable is unavailable",
            )
        return SandboxResourceAlertHostScenario(
            self._config,
            test_context,
            agent,
        ).run()

    def _repository_problem(self) -> TestResult | None:
        required = (
            self._config.bin_dir / "actraild",
            self._config.bin_dir / "actrail-sb",
            self._config.bin_dir / "actrail-vsock-gateway",
            self._config.bin_dir / "actraild-alert-proxy",
            self._config.bin_dir / "actrailviewer",
            self._config.repo
            / "examples/plugins/builtin/sandbox-resource-alert/"
            "sandbox-resource-alert.plugin.toml",
            self._config.repo
            / "examples/plugins/builtin/sandbox-resource-alert/"
            "sandbox-resource-alert.config.json",
        )
        missing = [path for path in required if not path.is_file()]
        non_executable = [
            path
            for path in required[:5]
            if path.is_file() and not os.access(path, os.X_OK)
        ]
        if not missing and not non_executable:
            return None
        return TestResult(
            TestStatus.FAILED,
            "host sandbox alert release/assets are unavailable: "
            + ", ".join(str(path) for path in (*missing, *non_executable)),
        )

    @staticmethod
    def _host_problem() -> TestResult | None:
        reasons = []
        for path in (Path("/dev/vsock"), Path("/sys/module/vsock_loopback")):
            if not path.exists():
                reasons.append(f"{path} is unavailable")
        if not Path("/sys/kernel/btf/vmlinux").is_file():
            reasons.append("kernel BTF is unavailable")
        if not reasons:
            return None
        return TestResult(
            TestStatus.SKIPPED,
            "host-native sandbox collection prerequisite unavailable: "
            + "; ".join(reasons),
        )
