from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.kata_runtime import (
    DeploymentArtifacts,
    KataContainerRequirements,
    KataMount,
    KataRequirementsBuilder,
    KataTestContainer,
    RequirementCheck,
)
from tests.v2.common.process import SubprocessRunner

from ..asset_bundle import CloudHypervisorAssetBundle
from ..config import CloudHypervisorExecutionIsolationConfig
from .transport import (
    CoordinationDirectory,
    FirecrackerAssetTransport,
    GuestCoordination,
    HostCoordination,
)


class CloudHypervisorScenarioSetup:
    def __init__(
        self,
        config: CloudHypervisorExecutionIsolationConfig,
        deployment: DeploymentArtifacts,
        runner: SubprocessRunner,
        run_dir: Path,
        assets: Path,
        coordination: Path,
    ) -> None:
        self._config = config
        self._deployment = deployment
        self._runner = runner
        self._run_dir = run_dir
        self._assets = assets
        self._coordination = coordination

    def prepare_assets(self) -> None:
        identity = self._config.IDENTITY
        assert self._deployment.xiaoo is not None
        self._assets.mkdir(parents=True)
        if self._config.BACKEND != "firecracker":
            self._coordination.mkdir(mode=0o770)
            os.chown(
                self._coordination,
                self._config.workload_uid,
                self._config.workload_gid,
            )
        source_assets = Path(__file__).parent.parent / "assets"
        sources = {
            "xiaoo-real": self._deployment.xiaoo,
            "xiaoo-root": source_assets / "named_agent_root.py",
            "provider_proxy.py": (
                self._config.repo
                / "tests/support/llm-http-proxy/provider_proxy.py"
            ),
            "workload.sh": source_assets / "workload.sh",
            "oom-trigger.sh": source_assets / "oom-trigger.sh",
            "oom_trigger.py": source_assets / "oom_trigger.py",
        }
        for name, source in sources.items():
            if not source.is_file():
                raise RuntimeError(
                    identity.failure(
                        f"asset is missing: {source}"
                    )
                )
            shutil.copy2(source, self._assets / name)
        for name in (
            "xiaoo-real",
            "xiaoo-root",
            "workload.sh",
            "oom-trigger.sh",
            "oom_trigger.py",
        ):
            (self._assets / name).chmod(0o755)
        (self._assets / "xiaoo.toml").write_text(
            CloudHypervisorAssetBundle.xiaoo_config(),
            encoding="utf-8",
        )
        (self._assets / "task.txt").write_text(
            f"{identity.AGENT_READ_INPUT_MARKER}\n",
            encoding="utf-8",
        )
        self._write_plugin_assets()
        CloudHypervisorAssetBundle.write_manifest(self._assets)

    def requirements(self) -> KataContainerRequirements:
        firecracker = self._config.BACKEND == "firecracker"
        builder = KataRequirementsBuilder(
            backend=self._config.BACKEND,
            runtime=self._config.ctr_runtime,
            runtime_config=self._deployment.data_config,
            image=self._deployment.workload_image,
            runner=self._runner,
            pull_policy=self._config.image_pull_policy,
            image_archive=(
                self._deployment.workload_image_archive
                if firecracker
                else self._config.image_archive
            ),
            runtime_timeout_seconds=self._config.runtime_timeout_seconds,
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            ready_timeout_seconds=self._config.ready_timeout_seconds,
            snapshotter="devmapper" if firecracker else None,
            image_archive_sha256=(
                getattr(
                    self._deployment,
                    "workload_image_archive_sha256",
                    None,
                )
                if firecracker
                else None
            ),
        )
        if firecracker:
            # Kata Firecracker has no shared-filesystem transport. The Guest
            # observer is controlled through the debug console, and workload
            # assets/coordination are transported with ``ctr exec``.
            mounts = ()
            artifact_directories = (self._assets,)
            # Kata Firecracker truncates its sandbox ID to 32 bytes. Combined
            # with KataTestContainer's random-token-first ID, this short prefix
            # keeps the complete owned ID within that limit.
            name_prefix = "fc-alert"
        else:
            mounts = (
                KataMount(
                    self._deployment.workload_bundle,
                    "/opt/actrail",
                    read_only=True,
                ),
                KataMount(
                    self._deployment.guest_bundle,
                    "/opt/actrail-guest",
                    read_only=True,
                ),
                KataMount(
                    self._assets,
                    "/opt/actrail-execution",
                    read_only=True,
                ),
                KataMount(
                    self._coordination,
                    "/run/actrail-execution",
                    read_only=False,
                ),
                KataMount(Path("/dev/actrail"), "/run/actrail", read_only=True),
            )
            artifact_directories = (
                self._deployment.workload_bundle,
                self._deployment.guest_bundle,
                self._assets,
            )
            name_prefix = self._config.IDENTITY.CASE
        return builder.build(
            name_prefix=name_prefix,
            command=("/bin/sh", "-c", "sleep 600"),
            mounts=mounts,
            artifact_directories=artifact_directories,
            labels=(("io.actrail.test.case", self._config.IDENTITY.CASE),),
            running_validator=self._validate_vm_ready,
        )

    def stage_assets(self, vm: KataTestContainer) -> None:
        if self._config.BACKEND != "firecracker":
            return
        FirecrackerAssetTransport(
            self._assets,
            self._config.workload_uid,
            self._config.workload_gid,
            self._config.runtime_timeout_seconds,
        ).stage(vm)

    def coordination(self, vm: KataTestContainer) -> CoordinationDirectory:
        if self._config.BACKEND != "firecracker":
            return HostCoordination(self._coordination)
        return GuestCoordination(
            vm,
            self._config.workload_uid,
            self._config.workload_gid,
            self._config.ready_timeout_seconds,
        )

    def load_plugin(self, runtime: ActrailRuntime) -> None:
        identity = self._config.IDENTITY
        instance = identity.PLUGIN_INSTANCE
        result = runtime.run_checked(
            [
                runtime.actraild,
                "--config",
                self._run_dir / "actraild.conf",
                "plugin",
                "load",
                "--manifest",
                self._deployment.host_bundle
                / "sandbox-resource-alert/sandbox-resource-alert.plugin.toml",
                "--plugin-config",
                self._run_dir / "sandbox-resource-alert.json",
                "--instance",
                instance,
            ]
        )
        if f"loaded instance={instance}" not in result.output:
            raise RuntimeError(
                identity.failure(
                    "sandbox resource alert plugin did not become active"
                )
            )

    def write_gateway_config(
        self,
        socket_path: Path | None,
        daemon_port: int,
    ) -> None:
        if self._config.BACKEND == "cloud-hypervisor":
            if socket_path is None:
                raise ValueError("Cloud Hypervisor gateway socket is required")
            backend_arguments = (
                "--backend",
                "cloud-hypervisor",
                "--socket-path",
                str(socket_path),
            )
        elif self._config.BACKEND == "stratovirt":
            if socket_path is not None:
                raise ValueError("StratoVirt uses native AF_VSOCK, not a UDS")
            backend_arguments = (
                "--backend",
                "native",
                "--cid",
                "4294967295",
                "--port",
                str(self._config.vsock_port),
            )
        elif self._config.BACKEND == "firecracker":
            if socket_path is None:
                raise ValueError("Firecracker VSOCK base UDS is required")
            backend_arguments = (
                "--backend",
                "firecracker",
                "--uds-path",
                str(socket_path),
                "--port",
                str(self._config.vsock_port),
            )
        else:
            raise ValueError(f"unsupported VMM backend: {self._config.BACKEND}")
        self._run_config_init(
            self._config.bin_dir / "actrail-vsock-gateway",
            (
                "--output",
                str(self._run_dir / "gateway.toml"),
                *backend_arguments,
                "--daemon-address",
                f"127.0.0.1:{daemon_port}",
            ),
        )

    def workload_environment(self) -> dict[str, str]:
        identity = self._config.IDENTITY
        return {
            "ACTRAIL_XIAOO_INSTANCE": identity.CASE,
            "ACTRAIL_XIAOO_BIN": "/opt/actrail-execution/xiaoo-root",
            "ACTRAIL_XIAOO_CONFIG": "/opt/actrail-execution/xiaoo.toml",
            "ACTRAIL_XIAOO_PROMPT": (
                "Use the Bash tool calls requested by the provider, then reply "
                f"with exactly {identity.XIAOO_RESPONSE_MARKER}."
            ),
            "ACTRAIL_XIAOO_RESPONSE_MARKER": identity.XIAOO_RESPONSE_MARKER,
            "ACTRAIL_XIAOO_COORD_DIR": "/run/actrail-execution",
            "ACTRAIL_XIAOO_PROVIDER_SCRIPT": (
                "/opt/actrail-execution/provider_proxy.py"
            ),
            "ACTRAIL_XIAOO_READY_TIMEOUT_SECONDS": str(
                self._config.ready_timeout_seconds
            ),
            "ACTRAIL_EXECUTION_ISOLATION_REAL_XIAOO": (
                "/opt/actrail-execution/xiaoo-real"
            ),
            "ACTRAIL_EXECUTION_ISOLATION_ROOT_PID_FILE": (
                "/run/actrail-execution/root.pid"
            ),
            "ACTRAIL_EXECUTION_ISOLATION_CHILD_RELEASE": (
                "/run/actrail-execution/child.release"
            ),
            "ACTRAIL_EXECUTION_ISOLATION_CHILD_TIMEOUT_SECONDS": str(
                self._config.ready_timeout_seconds
            ),
            "ACTRAIL_EXECUTION_ISOLATION_AGENT_READ_MARKER": (
                identity.AGENT_READ_INPUT_MARKER
            ),
            "ACTRAIL_EXECUTION_ISOLATION_AGENT_WRITE_MARKER": (
                identity.AGENT_WRITE_MARKER
            ),
            "ACTRAIL_EXECUTION_ISOLATION_NAMED_ROOT_MARKER": (
                identity.NAMED_ROOT_MARKER
            ),
            "ACTRAIL_EXECUTION_ISOLATION_AGENT_TOOLS_MARKER": (
                identity.AGENT_TOOLS_MARKER
            ),
        }

    def _validate_vm_ready(self, vm: KataTestContainer) -> RequirementCheck:
        if not vm.is_running():
            return RequirementCheck.not_ready(
                self._config.IDENTITY.failure(
                    "Kata task exited before readiness"
                ),
                refreshable=True,
            )
        firecracker = self._config.BACKEND == "firecracker"
        mounted_assets = ""
        if not firecracker:
            mounted_assets = (
                "test -x /opt/actrail-execution/xiaoo-real; "
                "test -x /opt/actrail-execution/xiaoo-root; "
                "/opt/actrail-execution/xiaoo-real --cli run --help "
                "2>&1 | grep -q -- --tools; "
            )
        result = vm.exec(
            (
                "/bin/sh",
                "-ec",
                ". /etc/os-release; [ \"${ID:-}\" = openEuler ]; "
                "test -r /sys/kernel/btf/vmlinux; "
                "command -v python3 >/dev/null; "
                f"{mounted_assets}"
            ),
            timeout=self._config.ready_timeout_seconds + 2,
        )
        if result.returncode == 0:
            return RequirementCheck.ready_check()
        return RequirementCheck.not_ready(
            self._config.IDENTITY.failure(
                result.diagnostic or "Guest readiness failed"
            ),
            refreshable=not any(
                marker in result.diagnostic.lower()
                for marker in ("permission denied", "operation not permitted", "kvm")
            ),
        )

    def _run_config_init(
        self,
        binary: Path,
        arguments: tuple[str, ...],
    ) -> None:
        result = self._runner.run(
            (str(binary), "init", *arguments),
            timeout=self._config.command_timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    "current release config generator failed: "
                    f"{result.diagnostic}"
                )
            )

    def _write_plugin_assets(self) -> None:
        default_config = (
            self._deployment.host_bundle
            / "sandbox-resource-alert/sandbox-resource-alert.config.json"
        )
        document = json.loads(default_config.read_text(encoding="utf-8"))
        document.update(
            {
                "cpu_usage_threshold_basis_points": 1,
                "memory_available_threshold_bytes": 18_446_744_073_709_551_615,
                "read_interval_threshold_bytes": 1,
                "write_interval_threshold_bytes": 1,
                "source_state_capacity": 64,
            }
        )
        (self._run_dir / "sandbox-resource-alert.json").write_text(
            json.dumps(document, indent=2) + "\n",
            encoding="utf-8",
        )
