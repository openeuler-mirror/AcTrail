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
            self._config.bin_dir / "actrailweb",
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
            for path in required[:6]
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
        cgroup_problem = SandboxResourceAlertHostCase._memory_cgroup_problem()
        if cgroup_problem is not None:
            reasons.append(cgroup_problem)
        if not reasons:
            return None
        return TestResult(
            TestStatus.SKIPPED,
            "host-native sandbox collection prerequisite unavailable: "
            + "; ".join(reasons),
        )

    @staticmethod
    def _memory_cgroup_problem() -> str | None:
        if Path("/sys/fs/cgroup/cgroup.controllers").is_file():
            parent = Path("/sys/fs/cgroup")
            limit_name = "memory.max"
        elif Path("/sys/fs/cgroup/memory/memory.limit_in_bytes").is_file():
            parent = Path("/sys/fs/cgroup/memory")
            limit_name = "memory.limit_in_bytes"
        else:
            return "memory cgroup controller is unavailable"
        probe = parent / f"actrail-precheck-{os.getpid()}"
        if probe.exists():
            return f"memory cgroup precheck path already exists: {probe}"
        try:
            probe.mkdir()
            limit = probe / limit_name
            if not limit.is_file():
                return f"memory controller is not enabled below {parent}"
            limit.write_text("33554432\n", encoding="ascii")
        except OSError as error:
            return f"memory cgroup is not delegated for the regression: {error}"
        finally:
            try:
                probe.rmdir()
            except OSError:
                pass
        return None
