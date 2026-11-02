from __future__ import annotations

import json
import os
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.kata_runtime import (
    DeploymentArtifacts,
    KataContainerRequirements,
    KataTestContainer,
    RequirementCheck,
)
from tests.v2.common.process import SubprocessRunner

from ..asset_bundle import ExecutionIsolationAssetBundle
from ..backends import ExecutionIsolationBackend
from ..config import KataExecutionIsolationConfig
from ..constants import CASE_CONSTANTS
from .transport import CoordinationDirectory


class ExecutionIsolationSetup:
    def __init__(
        self,
        config: KataExecutionIsolationConfig,
        deployment: DeploymentArtifacts,
        runner: SubprocessRunner,
        run_dir: Path,
        assets: Path,
        coordination: Path,
    ) -> None:
        self._config = config
        self._backend = ExecutionIsolationBackend(config.BACKEND)
        self._deployment = deployment
        self._runner = runner
        self._run_dir = run_dir
        self._assets = assets
        self._coordination = coordination

    def prepare_assets(self) -> None:
        identity = self._config.IDENTITY
        assert self._deployment.xiaoo is not None
        self._assets.mkdir(parents=True)
        if not self._backend.uses_guest_exec:
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
                / "tests/v2/common/llm-http-proxy/provider_proxy.py"
            ),
            "workload.sh": source_assets / "workload.sh",
            "oom-trigger.sh": source_assets / "oom-trigger.sh",
            "oom_trigger.py": source_assets / "oom_trigger.py",
            "constants.py": source_assets.parent / "constants.py",
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
            ExecutionIsolationAssetBundle.xiaoo_config(),
            encoding="utf-8",
        )
        (self._assets / "task.txt").write_text(
            f"{identity.AGENT_READ_INPUT_MARKER}\n",
            encoding="utf-8",
        )
        self._write_plugin_assets()
        ExecutionIsolationAssetBundle.write_manifest(self._assets)

    def requirements(self) -> KataContainerRequirements:
        return self._backend.requirements(
            self._config,
            self._deployment,
            self._runner,
            self._assets,
            self._coordination,
            self._validate_vm_ready,
        )

    def stage_assets(self, vm: KataTestContainer) -> None:
        self._backend.stage_assets(vm, self._config, self._assets)

    def coordination(self, vm: KataTestContainer) -> CoordinationDirectory:
        return self._backend.coordination(vm, self._config, self._coordination)

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
        backend_arguments = self._backend.gateway_arguments(
            socket_path, self._config.vsock_port,
        )
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
            "ACTRAIL_XIAOO_PROVIDER_PORT": str(CASE_CONSTANTS.provider_port),
            "ACTRAIL_VIRTUAL_XIAOO_API_KEY": CASE_CONSTANTS.local_api_key,
            "ACTRAIL_XIAOO_POLL_SECONDS": str(CASE_CONSTANTS.guest_poll_seconds),
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
        mounted_assets = self._backend.readiness_assets()
        result = vm.exec(
            (
                "/bin/sh",
                "-ec",
                ". /etc/os-release; [ \"${ID:-}\" = openEuler ]; "
                "test -r /sys/kernel/btf/vmlinux; "
                "command -v python3 >/dev/null; "
                f"{mounted_assets}"
            ),
            timeout=self._config.ready_timeout_seconds + CASE_CONSTANTS.command_timeout_slack_seconds,
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
                "cpu_usage_threshold_basis_points": CASE_CONSTANTS.resource_alert_cpu_basis_points,
                "memory_available_threshold_bytes": 18_446_744_073_709_551_615,
                "read_interval_threshold_bytes": CASE_CONSTANTS.resource_alert_io_threshold_bytes,
                "write_interval_threshold_bytes": CASE_CONSTANTS.resource_alert_io_threshold_bytes,
                "source_state_capacity": CASE_CONSTANTS.resource_alert_source_capacity,
            }
        )
        (self._run_dir / "sandbox-resource-alert.json").write_text(
            json.dumps(document, indent=2) + "\n",
            encoding="utf-8",
        )
