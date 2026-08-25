from __future__ import annotations

import os
import shutil
from pathlib import Path

from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.execution_isolation.controlled_oom import (
    memory_cgroup_problem,
)
from tests.v2.common.runner import TestingContextSingleton

from .config import SandboxOomKilledAlertHostConfig


class SandboxOomKilledAlertHostCase(TestCase):
    """Public case seam for the focused OOM-killed alert scenario."""

    def __init__(self, config: SandboxOomKilledAlertHostConfig) -> None:
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        problem = self._repository_problem()
        if problem is not None:
            return problem
        problem = self._host_problem()
        if problem is not None:
            return problem

        from .scenario import SandboxOomKilledAlertHostScenario

        return SandboxOomKilledAlertHostScenario(
            self._config,
            test_context,
        ).run()

    def _repository_problem(self) -> TestResult | None:
        bin_dir = (
            self._config.bin_dir
            if self._config.bin_dir.is_absolute()
            else self._config.repo / self._config.bin_dir
        )
        required_binaries = tuple(
            bin_dir / name
            for name in (
                "actraild",
                "actrail-sb",
                "actrail-vsock-gateway",
                "actraild-alert-proxy",
                "actrailviewer",
            )
        )
        required_assets = (
            self._config.repo
            / "examples/plugins/builtin/sandbox-resource-alert/"
            "sandbox-resource-alert.plugin.toml",
            self._config.repo
            / "examples/plugins/builtin/sandbox-resource-alert/"
            "sandbox-resource-alert.config.json",
        )
        missing = [
            path
            for path in (*required_binaries, *required_assets)
            if not path.is_file()
        ]
        non_executable = [
            path
            for path in required_binaries
            if path.is_file() and not os.access(path, os.X_OK)
        ]
        if not missing and not non_executable:
            return None
        return TestResult(
            TestStatus.FAILED,
            "focused host OOM alert release/assets are unavailable: "
            + ", ".join(str(path) for path in (*missing, *non_executable)),
        )

    @staticmethod
    def _host_problem() -> TestResult | None:
        reasons = []
        for path in (
            Path("/dev/vsock"),
            Path("/sys/module/vsock_loopback"),
        ):
            if not path.exists():
                reasons.append(f"{path} is unavailable")
        if not Path("/sys/kernel/btf/vmlinux").is_file():
            reasons.append("kernel BTF is unavailable")
        if not Path("/proc/vmstat").is_file():
            reasons.append("kernel vmstat is unavailable")
        if not os.access("/bin/sh", os.X_OK):
            reasons.append("/bin/sh is unavailable")
        for command in ("awk", "python3"):
            if shutil.which(command) is None:
                reasons.append(f"{command} is unavailable")
        cgroup_problem = memory_cgroup_problem()
        if cgroup_problem is not None:
            reasons.append(cgroup_problem)
        if not reasons:
            return None
        return TestResult(
            TestStatus.SKIPPED,
            "focused host OOM alert prerequisite unavailable: "
            + "; ".join(reasons),
        )
