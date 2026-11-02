"""Backend differences for the shared Kata execution-isolation scenario.

Lifecycle and observation assertions stay in the scenario. This adapter owns
VSOCK endpoints, workload delivery, and snapshotter prerequisites.
"""

from __future__ import annotations

import shutil
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.kata_runtime import (
    DeploymentArtifacts,
    KataContainerRequirements,
    KataMount,
    KataRequirementsBuilder,
    KataTestContainer,
    RequirementCheck,
)
from tests.v2.common.kata_runtime.backend import kata_backend
from tests.v2.common.process import CommandRunner, SubprocessRunner

from .config import KataExecutionIsolationConfig
from .constants import CASE_CONSTANTS
from .scenario.transport import (
    CoordinationDirectory,
    FirecrackerAssetTransport,
    GuestCoordination,
    HostCoordination,
)
from .vsock import (
    CloudHypervisorSocketInventory,
    FirecrackerSocketInventory,
    HybridVsockSocketInventory,
)


@dataclass(frozen=True)
class ExecutionIsolationBackend:
    name: str

    def __post_init__(self) -> None:
        kata_backend(self.name)

    @property
    def uses_guest_exec(self) -> bool:
        return not kata_backend(self.name).supports_shared_filesystem

    @property
    def snapshotter(self) -> str | None:
        return kata_backend(self.name).default_snapshotter

    def socket_inventory(self, vm_root: Path) -> HybridVsockSocketInventory | None:
        inventory_type = {
            "cloud-hypervisor": CloudHypervisorSocketInventory,
            "firecracker": FirecrackerSocketInventory,
        }.get(self.name)
        return None if inventory_type is None else inventory_type(vm_root)

    def gateway_arguments(self, socket_path: Path | None, port: int) -> tuple[str, ...]:
        if self.name == "cloud-hypervisor":
            if socket_path is None:
                raise ValueError("Cloud Hypervisor gateway socket is required")
            return (
                "--backend",
                "cloud-hypervisor",
                "--socket-path",
                str(socket_path),
            )
        elif self.name == "stratovirt":
            if socket_path is not None:
                raise ValueError("StratoVirt uses native AF_VSOCK, not a UDS")
            return (
                "--backend",
                "native",
                "--cid",
                "4294967295",
                "--port",
                str(port),
            )
        elif self.name == "firecracker":
            if socket_path is None:
                raise ValueError("Firecracker VSOCK base UDS is required")
            return (
                "--backend",
                "firecracker",
                "--uds-path",
                str(socket_path),
                "--port",
                str(port),
            )
        else:
            raise ValueError(f"unsupported VMM backend: {self.name}")

    def requirements(
        self,
        config: KataExecutionIsolationConfig,
        deployment: DeploymentArtifacts,
        runner: SubprocessRunner,
        assets: Path,
        coordination: Path,
        running_validator: Callable[[KataTestContainer], RequirementCheck],
    ) -> KataContainerRequirements:
        guest_exec = self.uses_guest_exec
        builder = KataRequirementsBuilder(
            backend=config.BACKEND,
            runtime=config.ctr_runtime,
            runtime_config=deployment.data_config,
            image=deployment.workload_image,
            runner=runner,
            pull_policy=config.image_pull_policy,
            image_archive=(
                deployment.workload_image_archive
                if guest_exec
                else config.image_archive
            ),
            runtime_timeout_seconds=config.runtime_timeout_seconds,
            uid=config.workload_uid,
            gid=config.workload_gid,
            ready_timeout_seconds=config.ready_timeout_seconds,
            snapshotter=self.snapshotter,
            image_archive_sha256=(
                getattr(
                    deployment,
                    "workload_image_archive_sha256",
                    None,
                )
                if guest_exec
                else None
            ),
        )
        if guest_exec:
            # Kata Firecracker has no shared-filesystem transport. The Guest
            # observer is controlled through the debug console, and workload
            # assets/coordination are transported with ``ctr exec``.
            mounts = ()
            artifact_directories = (assets,)
            # Kata Firecracker truncates its sandbox ID to 32 bytes. Combined
            # with KataTestContainer's random-token-first ID, this short prefix
            # keeps the complete owned ID within that limit.
            name_prefix = "fc-alert"
        else:
            mounts = (
                KataMount(
                    deployment.workload_bundle,
                    "/opt/actrail",
                    read_only=True,
                ),
                KataMount(
                    deployment.guest_bundle,
                    "/opt/actrail-guest",
                    read_only=True,
                ),
                KataMount(
                    assets,
                    "/opt/actrail-execution",
                    read_only=True,
                ),
                KataMount(
                    coordination,
                    "/run/actrail-execution",
                    read_only=False,
                ),
                KataMount(Path("/dev/actrail"), "/run/actrail", read_only=True),
            )
            artifact_directories = (
                deployment.workload_bundle,
                deployment.guest_bundle,
                assets,
            )
            name_prefix = config.IDENTITY.CASE
        return builder.build(
            name_prefix=name_prefix,
            command=("/bin/sh", "-c", f"sleep {CASE_CONSTANTS.vm_hold_seconds}"),
            mounts=mounts,
            artifact_directories=artifact_directories,
            labels=(("io.actrail.test.case", config.IDENTITY.CASE),),
            running_validator=running_validator,
        )

    def stage_assets(
        self,
        vm: KataTestContainer,
        config: KataExecutionIsolationConfig,
        assets: Path,
    ) -> None:
        if self.uses_guest_exec:
            FirecrackerAssetTransport(
                assets,
                config.workload_uid,
                config.workload_gid,
                config.runtime_timeout_seconds,
            ).stage(vm)

    def coordination(
        self,
        vm: KataTestContainer,
        config: KataExecutionIsolationConfig,
        root: Path,
    ) -> CoordinationDirectory:
        if self.uses_guest_exec:
            return GuestCoordination(
                vm,
                config.workload_uid,
                config.workload_gid,
                config.ready_timeout_seconds,
            )
        return HostCoordination(root)

    def readiness_assets(self) -> str:
        if self.uses_guest_exec:
            return ""
        return (
            "test -x /opt/actrail-execution/xiaoo-real; "
            "test -x /opt/actrail-execution/xiaoo-root; "
            "/opt/actrail-execution/xiaoo-real --cli run --help "
            "2>&1 | grep -q -- --tools; "
        )

    def external_problem(self, runner: CommandRunner, timeout_seconds: float) -> str | None:
        if self.snapshotter != "devmapper":
            return None
        if shutil.which("dmsetup") is None:
            return "dmsetup is unavailable for the Firecracker devmapper snapshotter"
        plugin_type = "io.containerd.snapshotter.v1"
        try:
            result = runner.run(
                (
                    "ctr",
                    "plugins",
                    "list",
                    f"type=={plugin_type},id==devmapper",
                ),
                timeout=timeout_seconds,
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
