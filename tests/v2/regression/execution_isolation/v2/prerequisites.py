from __future__ import annotations

import os
import platform
import shutil
from dataclasses import dataclass

from tests.v2.common.kata_runtime import DeploymentArtifacts, shim_binary
from tests.v2.common.core import TestResult, TestStatus

from .config import ExecutionIsolationConfig


@dataclass(frozen=True)
class ExecutionIsolationReadiness:
    deployment: DeploymentArtifacts | None
    problem: TestResult | None


class ExecutionIsolationPrerequisites:
    """Fail-fast repository checks and skip-only external capability checks."""

    def __init__(self, config: ExecutionIsolationConfig) -> None:
        self._config = config

    def resolve(self) -> ExecutionIsolationReadiness:
        release_problem = self._release_problem()
        if release_problem is not None:
            return ExecutionIsolationReadiness(None, release_problem)
        deployment, deployment_problem = self._deployment()
        if deployment_problem is not None:
            return ExecutionIsolationReadiness(None, deployment_problem)
        external_problem = self._external_problem()
        return ExecutionIsolationReadiness(deployment, external_problem)

    def _release_problem(self) -> TestResult | None:
        required = (
            "actraild",
            "actrail-sb",
            "actrail-vsock-gateway",
        )
        missing = [
            self._config.bin_dir / name
            for name in required
            if not (
                (self._config.bin_dir / name).is_file()
                and os.access(self._config.bin_dir / name, os.X_OK)
            )
        ]
        if not missing:
            return None
        return TestResult(
            TestStatus.FAILED,
            "current release is missing execution-isolation binary artifact(s): "
            + ", ".join(str(path) for path in missing),
        )

    def _deployment(
        self,
    ) -> tuple[DeploymentArtifacts | None, TestResult | None]:
        manifest = self._config.artifact_manifest
        if manifest is None:
            return None, TestResult(
                TestStatus.FAILED,
                "EXECUTION_ISOLATION_E2E_ARTIFACT_MANIFEST is required; "
                "refresh the V2 content-addressed test profile",
            )
        try:
            deployment = DeploymentArtifacts.load(
                manifest,
                bin_dir=self._config.bin_dir,
                expected_backend="cloud-hypervisor",
                expected_runtime=self._config.ctr_runtime,
                require_xiaoo=True,
            )
        except (OSError, RuntimeError, ValueError) as error:
            return None, TestResult(
                TestStatus.FAILED,
                f"execution-isolation artifact manifest is invalid: {error}",
            )
        if self._config.runtime_config is not None:
            configured = self._config.runtime_config.resolve()
            if configured != deployment.data_config.resolve():
                return None, TestResult(
                    TestStatus.FAILED,
                    "configured runtime config does not match the refreshed "
                    "manifest data config",
                )
        if (
            self._config.xiaoo_binary is not None
            and self._config.xiaoo_binary.resolve() != deployment.xiaoo
        ):
            return None, TestResult(
                TestStatus.FAILED,
                "configured xiaoO binary does not match the refreshed manifest",
            )
        if self._config.image != deployment.workload_image:
            return None, TestResult(
                TestStatus.FAILED,
                "configured workload image does not match the refreshed manifest",
            )
        return deployment, None

    def _external_problem(self) -> TestResult | None:
        reasons: list[str] = []
        if platform.machine() != "aarch64":
            reasons.append("host architecture is not aarch64")
        if not os.access("/dev/kvm", os.R_OK | os.W_OK):
            reasons.append("/dev/kvm is not readable and writable")
        for command in (
            "ctr",
            "kata-runtime",
            "script",
            shim_binary(self._config.ctr_runtime),
            "cloud-hypervisor",
        ):
            if shutil.which(command) is None:
                reasons.append(f"{command} is unavailable")
        if not reasons:
            return None
        return TestResult(
            TestStatus.SKIPPED,
            "external Cloud Hypervisor prerequisite unavailable: "
            + "; ".join(reasons),
        )
