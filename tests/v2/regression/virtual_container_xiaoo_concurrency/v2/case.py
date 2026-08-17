from __future__ import annotations

import os
import re
import shutil
from pathlib import Path

from tests.v2.common.kata_runtime import (
    DeploymentArtifacts,
    kata_backend,
    resolve_deployment_artifacts,
    shim_binary,
)
from tests.v2.common.kata_runtime.process import SubprocessRunner
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .config import VirtualContainerXiaooConcurrencyConfig
from .scenario import DualKataXiaooScenario


class VirtualContainerXiaooConcurrencyCase(TestCase):
    def __init__(self, config: VirtualContainerXiaooConcurrencyConfig):
        self._config = config
        self._validator = (
            config.repo
            / "tests/v2/regression/virtual_container/validate-runtime-config.py"
        )
        self._provider = (
            config.repo / "tests/support/llm-http-proxy/provider_proxy.py"
        )
        self._workload = Path(__file__).resolve().parent / "workload.sh"

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        problem = self._host_prerequisite_problem()
        if problem is not None:
            return problem
        test_context.report_progress(
            "artifacts",
            "validating concurrency deployment assets",
        )
        deployment, problem = self._resolve_deployment()
        if problem is not None:
            return problem
        problem = self._prerequisite_problem(deployment)
        if problem is not None:
            return problem
        runtime_config = (
            deployment.data_config
            if deployment is not None
            else self._config.runtime_config
        )
        assert runtime_config is not None

        test_context.report_progress(
            "preflight",
            "validating data Profile and guest kernel capabilities",
        )
        validation = SubprocessRunner().run(
            [
                str(self._validator),
                "--backend",
                self._config.backend,
                "--require-kernel-config",
                "--require-ebpf",
                str(runtime_config),
            ],
            timeout=self._config.command_timeout_seconds,
        )
        test_context.output.command_output(validation.stdout, validation.stderr)
        if validation.returncode != 0:
            return TestResult(
                TestStatus.FAILED,
                "Kata data runtime configuration is invalid",
            )
        return DualKataXiaooScenario(
            self._config,
            test_context,
            deployment,
        ).run()

    def _resolve_deployment(
        self,
    ) -> tuple[DeploymentArtifacts | None, TestResult | None]:
        manifest = self._config.artifact_manifest
        try:
            deployment = resolve_deployment_artifacts(
                manifest,
                bin_dir=self._config.bin_dir,
                guest_bundle=self._config.capability_bundle,
                workload_bundle=self._config.workload_bundle,
                expected_backend=self._config.backend,
                expected_runtime=self._config.ctr_runtime,
                expected_workload_image=self._config.image,
            )
        except (OSError, RuntimeError, ValueError) as error:
            return None, _deployment_failure(str(error))
        return deployment, None

    def _host_prerequisite_problem(self) -> TestResult | None:
        if not os.access("/dev/kvm", os.R_OK | os.W_OK):
            return TestResult(
                TestStatus.SKIPPED,
                "external Kata prerequisite is unavailable: "
                "readable/writable /dev/kvm",
            )
        commands = (
            "ctr",
            "kata-runtime",
            "script",
            shim_binary(self._config.ctr_runtime),
            kata_backend(self._config.backend).vmm_command,
        )
        unavailable = next(
            (command for command in commands if shutil.which(command) is None),
            None,
        )
        if unavailable is not None:
            return TestResult(
                TestStatus.SKIPPED,
                f"external Kata prerequisite is unavailable: {unavailable}",
            )
        return None

    def _prerequisite_problem(
        self,
        deployment: DeploymentArtifacts | None,
    ) -> TestResult | None:
        runtime_config = (
            deployment.data_config
            if deployment is not None
            else self._config.runtime_config
        )
        workload_bundle = (
            deployment.workload_bundle
            if deployment is not None
            else self._config.workload_bundle
        )
        xiaoo_binary = (
            deployment.xiaoo
            if deployment is not None
            else self._config.xiaoo_binary
        )
        project_files = (
            self._provider,
            self._workload,
            self._validator,
            workload_bundle / "MANIFEST.sha256",
        )
        missing = [path for path in project_files if not path.is_file()]
        if runtime_config is None:
            missing.append(Path("<Kata data runtime configuration>"))
        elif not runtime_config.is_file():
            missing.append(runtime_config)
        if missing:
            return TestResult(
                TestStatus.FAILED,
                "required deployment artifact(s) missing: "
                + ", ".join(str(path) for path in missing),
            )
        if xiaoo_binary is None or not (
            xiaoo_binary.is_file() and os.access(xiaoo_binary, os.X_OK)
        ):
            return TestResult(
                TestStatus.SKIPPED,
                "external xiaoO prerequisite is unavailable: "
                f"{xiaoo_binary or '<not configured>'}",
            )

        assert runtime_config is not None
        try:
            content = runtime_config.read_text(encoding="utf-8")
        except OSError as error:
            return TestResult(TestStatus.FAILED, str(error))
        if re.search(
            r"(?m)^[ \t]*debug_console_enabled[ \t]*=[ \t]*true[ \t]*$",
            content,
        ) is None:
            return TestResult(
                TestStatus.FAILED,
                "Kata data config must enable debug_console_enabled",
            )
        vcpu = re.search(
            r"(?m)^[ \t]*default_vcpus[ \t]*=[ \t]*([0-9]+)[ \t]*$",
            content,
        )
        if vcpu is None or int(vcpu.group(1)) < 2:
            return TestResult(
                TestStatus.FAILED,
                "each concurrent eBPF Kata VM requires default_vcpus >= 2",
            )
        return None


def _deployment_failure(reason: str) -> TestResult:
    return TestResult(
        TestStatus.FAILED,
        reason
        + "; rerun deploy/virtual-container/host/"
        "prepare-v2-test-artifacts.py",
    )
