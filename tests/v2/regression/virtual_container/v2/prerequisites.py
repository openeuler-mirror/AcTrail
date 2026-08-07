from __future__ import annotations

import os
import re
import shutil
from dataclasses import dataclass
from pathlib import Path
from typing import Protocol

from tests.v2.common.kata_runtime import (
    DeploymentArtifacts,
    kata_backend,
    resolve_deployment_artifacts,
    shim_binary,
)
from tests.v2.common.core import TestResult, TestStatus

from .config import VirtualContainerConfig


class HostProbe(Protocol):
    def command_path(self, name: str) -> Path | None: ...

    def kvm_available(self) -> bool: ...


class LocalHostProbe:
    def command_path(self, name: str) -> Path | None:
        resolved = shutil.which(name)
        return Path(resolved) if resolved else None

    def kvm_available(self) -> bool:
        return os.access("/dev/kvm", os.R_OK | os.W_OK)


@dataclass(frozen=True)
class ResolvedBackend:
    name: str
    runtime_config: Path | None
    data_config: Path | None
    problem: TestResult | None = None


class VirtualContainerPrerequisites:
    def __init__(
        self,
        config: VirtualContainerConfig,
        host: HostProbe | None = None,
    ) -> None:
        self._config = config
        self._host = host or LocalHostProbe()

    def release_problem(self) -> TestResult | None:
        required = (
            "actraild",
            "actrailctl",
            "actrailviewer",
            "libactrail_tls_payload_probe_sync.so",
        )
        missing = [
            self._config.bin_dir / name
            for name in required
            if not (self._config.bin_dir / name).is_file()
        ]
        if not missing:
            return None
        return TestResult(
            TestStatus.FAILED,
            "required AcTrail release artifact(s) missing: "
            + ", ".join(str(path) for path in missing),
        )

    def kvm_available(self) -> bool:
        return self._host.kvm_available()

    def resolve_deployment(
        self,
    ) -> tuple[DeploymentArtifacts | None, TestResult | None]:
        manifest = self._config.artifact_manifest
        if manifest is None:
            expected_backend = self._config.backends[0]
        else:
            if len(self._config.backends) != 1:
                return None, _deployment_failure(
                    "one content-addressed manifest currently describes one backend; "
                    "select exactly one VIRTUAL_CONTAINER_E2E_BACKENDS value"
                )
            expected_backend = self._config.backends[0]
        try:
            deployment = resolve_deployment_artifacts(
                manifest,
                bin_dir=self._config.bin_dir,
                guest_bundle=self._config.capability_bundle,
                workload_bundle=self._config.workload_bundle,
                expected_backend=expected_backend,
                expected_runtime=self._config.ctr_runtime,
                expected_workload_image=self._config.image,
            )
        except RuntimeError as error:
            return None, _deployment_failure(str(error))
        return deployment, None

    def resolve_backends(
        self,
        deployment: DeploymentArtifacts | None = None,
    ) -> dict[str, ResolvedBackend]:
        return {
            backend: self._resolve_backend(backend, deployment)
            for backend in self._config.backends
        }

    def _resolve_backend(
        self,
        backend: str,
        deployment: DeploymentArtifacts | None,
    ) -> ResolvedBackend:
        runtime_config, runtime_problem = self._resolve_config(
            backend,
            data=False,
            deployment=deployment,
        )
        if runtime_problem is not None and runtime_problem.status is TestStatus.FAILED:
            return ResolvedBackend(backend, runtime_config, None, runtime_problem)
        data_config, data_problem = self._resolve_config(
            backend,
            data=True,
            deployment=deployment,
        )
        if data_problem is not None and data_problem.status is TestStatus.FAILED:
            return ResolvedBackend(
                backend,
                runtime_config,
                data_config,
                data_problem,
            )
        config_problem = runtime_problem or data_problem
        if config_problem is not None:
            return ResolvedBackend(
                backend,
                runtime_config,
                data_config,
                config_problem,
            )
        assert runtime_config is not None
        assert data_config is not None
        try:
            data_content = data_config.read_text(encoding="utf-8")
        except OSError as error:
            return ResolvedBackend(
                backend,
                runtime_config,
                data_config,
                TestResult(
                    TestStatus.FAILED,
                    f"cannot read Kata data config {data_config}: {error}",
                ),
            )
        if re.search(
            r"(?m)^[ \t]*debug_console_enabled[ \t]*=[ \t]*true[ \t]*$",
            data_content,
        ) is None:
            return ResolvedBackend(
                backend,
                runtime_config,
                data_config,
                TestResult(
                    TestStatus.FAILED,
                    f"Kata data config for {backend} must enable "
                    "debug_console_enabled",
                ),
            )
        external_commands = (
            "ctr",
            "kata-runtime",
            shim_binary(self._config.ctr_runtime),
            kata_backend(backend).vmm_command,
        )
        missing_command = next(
            (
                command
                for command in external_commands
                if self._host.command_path(command) is None
            ),
            None,
        )
        if missing_command is not None:
            return ResolvedBackend(
                backend,
                runtime_config,
                data_config,
                TestResult(
                    TestStatus.SKIPPED,
                    f"external Kata prerequisite is unavailable: {missing_command}",
                ),
            )
        if not self._host.kvm_available():
            return ResolvedBackend(
                backend,
                runtime_config,
                data_config,
                TestResult(
                    TestStatus.SKIPPED,
                    "external Kata prerequisite is unavailable: "
                    "readable/writable /dev/kvm",
                ),
            )
        return ResolvedBackend(backend, runtime_config, data_config)

    def _resolve_config(
        self,
        backend: str,
        *,
        data: bool,
        deployment: DeploymentArtifacts | None,
    ) -> tuple[Path | None, TestResult | None]:
        configured = self._config.runtime_config(backend, data=data)
        explicitly_configured = (
            self._config.data_configs[backend]
            if data
            else self._config.runtime_configs[backend]
        )
        if deployment is not None:
            if backend != deployment.backend:
                return None, TestResult(
                    TestStatus.FAILED,
                    f"artifact manifest does not contain backend {backend}",
                )
            manifest_config = (
                deployment.data_config if data else deployment.base_config
            )
            if (
                explicitly_configured is not None
                and explicitly_configured.resolve() != manifest_config
            ):
                return None, TestResult(
                    TestStatus.FAILED,
                    "legacy runtime config conflicts with artifact manifest: "
                    f"legacy={explicitly_configured} manifest={manifest_config}",
                )
            configured = manifest_config
        if configured is None:
            configured = next(
                (
                    directory / kata_backend(backend).default_config_name
                    for directory in self._config.kata_config_dirs
                    if (
                        directory / kata_backend(backend).default_config_name
                    ).is_file()
                ),
                None,
            )
        if configured is not None and configured.is_file():
            return configured, None
        profile = "data " if data else ""
        if explicitly_configured is not None:
            return configured, TestResult(
                TestStatus.FAILED,
                f"configured Kata {profile}runtime config is missing: {configured}",
            )
        return None, TestResult(
            TestStatus.SKIPPED,
            f"Kata {profile}runtime config is unavailable for {backend}",
        )


def _deployment_failure(reason: str) -> TestResult:
    return TestResult(
        TestStatus.FAILED,
        reason
        + "; rerun deploy/virtual-container/host/"
        "prepare-v2-test-artifacts.py",
    )
