from __future__ import annotations

import os
import platform
import shutil
from dataclasses import dataclass

from tests.v2.common.kata_runtime import DeploymentArtifacts, shim_binary
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.process import CommandRunner, SubprocessRunner

from .config import CloudHypervisorExecutionIsolationConfig


@dataclass(frozen=True)
class CloudHypervisorExecutionIsolationReadiness:
    deployment: DeploymentArtifacts | None
    problem: TestResult | None


class CloudHypervisorExecutionIsolationPrerequisites:
    """Fail-fast repository checks and skip-only external capability checks."""

    def __init__(
        self,
        config: CloudHypervisorExecutionIsolationConfig,
        runner: CommandRunner | None = None,
    ) -> None:
        self._config = config
        self._runner = runner or SubprocessRunner()

    def resolve(self) -> CloudHypervisorExecutionIsolationReadiness:
        release_problem = self._release_problem()
        if release_problem is not None:
            return CloudHypervisorExecutionIsolationReadiness(None, release_problem)
        deployment, deployment_problem = self._deployment()
        if deployment_problem is not None:
            return CloudHypervisorExecutionIsolationReadiness(
                None,
                deployment_problem,
            )
        external_problem = self._external_problem()
        return CloudHypervisorExecutionIsolationReadiness(
            deployment,
            external_problem,
        )

    def _release_problem(self) -> TestResult | None:
        required = (
            "actraild",
            "actraild-alert-proxy",
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
            self._config.IDENTITY.failure(
                "current release is missing binary artifact(s): "
                + ", ".join(str(path) for path in missing)
            ),
        )

    def _deployment(
        self,
    ) -> tuple[DeploymentArtifacts | None, TestResult | None]:
        manifest = self._config.artifact_manifest
        if manifest is None:
            return None, TestResult(
                TestStatus.FAILED,
                self._config.IDENTITY.failure(
                    f"{self._config.ENVIRONMENT_PREFIX}ARTIFACT_MANIFEST "
                    "is required; refresh the V2 content-addressed test profile"
                ),
            )
        try:
            deployment = DeploymentArtifacts.load(
                manifest,
                bin_dir=self._config.bin_dir,
                expected_backend=self._config.BACKEND,
                expected_runtime=self._config.ctr_runtime,
                require_xiaoo=True,
                require_preinstalled_xiaoo=(
                    self._config.BACKEND == "firecracker"
                ),
                require_sandbox_observer=True,
            )
        except (OSError, RuntimeError, ValueError) as error:
            return None, TestResult(
                TestStatus.FAILED,
                self._config.IDENTITY.failure(
                    f"artifact manifest is invalid: {error}"
                ),
            )
        if self._config.runtime_config is not None:
            configured = self._config.runtime_config.resolve()
            if configured != deployment.data_config.resolve():
                return None, TestResult(
                    TestStatus.FAILED,
                    self._config.IDENTITY.failure(
                        "configured runtime config does not match the refreshed "
                        "manifest data config"
                    ),
                )
        if (
            self._config.xiaoo_binary is not None
            and self._config.xiaoo_binary.resolve() != deployment.xiaoo
        ):
            return None, TestResult(
                TestStatus.FAILED,
                self._config.IDENTITY.failure(
                    "configured xiaoO binary does not match the refreshed manifest"
                ),
            )
        if self._config.image != deployment.workload_image:
            return None, TestResult(
                TestStatus.FAILED,
                self._config.IDENTITY.failure(
                    "configured workload image does not match the refreshed manifest"
                ),
            )
        if (
            self._config.BACKEND == "firecracker"
            and self._config.image_archive is not None
            and self._config.image_archive.resolve()
            != deployment.workload_image_archive
        ):
            return None, TestResult(
                TestStatus.FAILED,
                self._config.IDENTITY.failure(
                    "configured workload image archive does not match the "
                    "refreshed manifest"
                ),
            )
        return deployment, None

    def _external_problem(self) -> TestResult | None:
        reasons: list[str] = []
        architecture = platform.machine()
        if architecture not in self._config.SUPPORTED_ARCHITECTURES:
            supported = ", ".join(sorted(self._config.SUPPORTED_ARCHITECTURES))
            reasons.append(
                f"host architecture {architecture} is not one of: {supported}"
            )
        if not os.access("/dev/kvm", os.R_OK | os.W_OK):
            reasons.append("/dev/kvm is not readable and writable")
        for command in (
            "ctr",
            "kata-runtime",
            "script",
            shim_binary(self._config.ctr_runtime),
            self._config.VMM_COMMAND,
        ):
            if shutil.which(command) is None:
                reasons.append(f"{command} is unavailable")
        if self._config.BACKEND == "firecracker" and not reasons:
            devmapper_problem = self._firecracker_devmapper_problem()
            if devmapper_problem is not None:
                reasons.append(devmapper_problem)
        if not reasons:
            return None
        return TestResult(
            TestStatus.SKIPPED,
            self._config.IDENTITY.failure(
                "external prerequisite unavailable: " + "; ".join(reasons)
            ),
        )

    def _firecracker_devmapper_problem(self) -> str | None:
        if shutil.which("dmsetup") is None:
            return "dmsetup is unavailable for the Firecracker devmapper snapshotter"
        plugin_type = "io.containerd.snapshotter.v1"
        try:
            result = self._runner.run(
                (
                    "ctr",
                    "plugins",
                    "list",
                    f"type=={plugin_type},id==devmapper",
                ),
                timeout=self._config.command_timeout_seconds,
            )
        except (OSError, RuntimeError) as error:
            return f"cannot inspect the containerd devmapper snapshotter: {error}"
        if result.returncode != 0:
            diagnostic = result.diagnostic or f"exit={result.returncode}"
            return f"containerd devmapper snapshotter probe failed: {diagnostic}"
        matches: list[tuple[str, ...]] = []
        for line in result.stdout.splitlines():
            columns = tuple(line.split())
            if (
                len(columns) >= 4
                and columns[0] == plugin_type
                and columns[1] == "devmapper"
            ):
                matches.append(columns)
        if len(matches) != 1:
            return (
                "containerd devmapper snapshotter is not registered exactly once"
            )
        status = matches[0][-1].lower()
        if status != "ok":
            return (
                "containerd devmapper snapshotter is unavailable: "
                f"status={status}"
            )
        return None
