from __future__ import annotations

import json
import os
import secrets
import shutil
import socket
import time
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.alert_proxy import AlertProxyTestProfile, AlertSubscriberClient
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.kata_runtime import (
    CtrCapabilities,
    DeploymentArtifacts,
    KataContainerRequirements,
    KataMount,
    KataRequirementsBuilder,
    KataTestContainer,
    RequirementCheck,
    sha256_file,
)
from tests.v2.common.kata_runtime.process import ManagedProcess, SubprocessRunner
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.sandbox_alert_database import (
    SandboxAlertDatabase,
    SandboxAlertRecord,
)

from .cloud_hypervisor import CloudHypervisorSocketInventory
from .config import ExecutionIsolationConfig


class ExecutionIsolationScenario:
    _ALERT_CATEGORIES = (
        "sandbox.resource.high_cpu",
        "sandbox.resource.oom_killed",
        "sandbox.resource.oom_risk",
        "sandbox.process.high_read",
        "sandbox.process.high_write",
    )

    def __init__(
        self,
        config: ExecutionIsolationConfig,
        context: TestingContextSingleton,
        deployment: DeploymentArtifacts,
    ) -> None:
        self._config = config
        self._context = context
        self._deployment = deployment
        self._runner = SubprocessRunner()
        self._run_token = secrets.token_hex(8)
        self._run_dir = config.work_dir / f"run-{self._run_token}"
        self._assets = self._run_dir / "assets"
        self._coord = self._run_dir / "coord"
        self._alerts = SandboxAlertDatabase(
            self._run_dir / "data" / "sandbox-alerts.sqlite"
        )
        self._inventory = CloudHypervisorSocketInventory(config.vm_root)

    def run(self) -> TestResult:
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        proxy: AlertProxyTestProfile | None = None
        subscriber: AlertSubscriberClient | None = None
        vm: KataTestContainer | None = None
        gateway: ManagedProcess | None = None
        sb: ManagedProcess | None = None
        workload: ManagedProcess | None = None
        cleanup_errors: list[str] = []
        try:
            self._prepare_assets()
            daemon_port = resolve_test_port(
                "EXECUTION_ISOLATION_E2E_DAEMON_PORT"
            )
            subscriber_port = resolve_test_port(
                "EXECUTION_ISOLATION_E2E_ALERT_PROXY_PORT"
            )
            proxy = AlertProxyTestProfile.create(
                self._run_dir,
                self._deployment.host_bundle / "actraild-alert-proxy",
                subscriber_port,
                secrets.token_urlsafe(24),
            )
            proxy.write_forwarding_config(
                enabled=True,
                categories=list(self._ALERT_CATEGORIES),
            )
            runtime = ActrailRuntime.isolated(
                self._config.repo,
                self._config.bin_dir,
                self._config.command_timeout_seconds,
                self._context.output,
                self._run_dir,
                hand_observation_listen_addr=f"127.0.0.1:{daemon_port}",
                sandbox_alerts_database=self._run_dir
                / "data"
                / "sandbox-alerts.sqlite",
                alert_forwarding=proxy.runtime_paths,
                clean_control_state=False,
            )
            self._context.report_progress(
                "daemon",
                "starting isolated hand-observation listener and plugin",
            )
            runtime.prepare()
            proxy.require_running()
            subscriber = AlertSubscriberClient(
                proxy.subscriber_address,
                proxy.token,
                f"execution-isolation-{self._run_token}",
                self._run_dir / "alert-subscriber.jsonl",
            )
            subscriber.connect(self._config.ready_timeout_seconds)
            subscriber.subscribe(
                "execution-isolation-alerts",
                list(self._ALERT_CATEGORIES),
                [],
                self._config.ready_timeout_seconds,
            )
            subscriber.wait_for_heartbeat(self._config.ready_timeout_seconds)
            self._load_plugin(runtime)
            self._wait_tcp(daemon_port)
            results["daemon"] = TestResult(
                TestStatus.PASSED,
                "hand-observation listener and sandbox plugin are active",
            )

            before = self._inventory.snapshot()
            vm = KataTestContainer(
                self._requirements(),
                self._runner,
                CtrCapabilities.detect(self._runner),
            )
            self._context.report_progress(
                "vm",
                "starting one test-owned Cloud Hypervisor Kata VM",
            )
            vm.start()
            base_socket = self._inventory.resolve_new_base_socket(
                before,
                self._inventory.snapshot(),
            )
            gateway_socket = self._inventory.gateway_socket(
                base_socket,
                self._config.vsock_port,
            )
            self._write_gateway_config(gateway_socket, daemon_port)
            gateway = self._runner.start(
                (
                    str(
                        self._deployment.host_bundle
                        / "actrail-vsock-gateway"
                    ),
                    "--config",
                    str(self._run_dir / "gateway.toml"),
                )
            )
            self._require_alive(gateway, "gateway")
            results["gateway"] = TestResult(
                TestStatus.PASSED,
                f"gateway owns {gateway_socket}",
            )

            workload = vm.start_exec(
                (
                    "/bin/sh",
                    "/opt/actrail-execution/workload.sh",
                ),
                uid=self._config.workload_uid,
                gid=self._config.workload_gid,
                environment=self._workload_environment(),
            )
            self._wait_path(
                self._coord / "provider.ready",
                workload,
                "real xiaoO provider readiness",
            )
            sb = vm.start_exec(
                (
                    "/opt/actrail-guest/actrail-sb",
                    "--config",
                    "/opt/actrail-execution/sb.toml",
                ),
                uid=0,
                gid=0,
                environment={"LD_LIBRARY_PATH": "/opt/actrail-guest/lib"},
            )
            self._require_alive(sb, "actrail-sb")
            self._wait_resource_baseline(gateway, sb)
            self._trigger_guest_oom(vm)
            (self._coord / "release").touch()
            self._wait_path(
                self._coord / "root.pid",
                workload,
                "named Agent root discovery",
            )
            root_pid = self._read_root_pid()
            time.sleep(self._config.root_discovery_settle_seconds)
            (self._coord / "child.release").touch()
            workload_result = workload.wait(
                timeout=self._config.runtime_timeout_seconds,
                terminate_grace_seconds=5,
            )
            workload = None
            output = workload_result.stdout + workload_result.stderr
            if workload_result.returncode != 0:
                raise RuntimeError(
                    "real xiaoO workload failed: " + workload_result.diagnostic
                )
            for marker in (
                "KATA_XIAOO_PROVIDER_READY instance=execution-isolation",
                "ACTRAIL_EXECUTION_ISOLATION_XIAOO_OK",
                "ACTRAIL_EXECUTION_NAMED_ROOT_OK",
                "ACTRAIL_EXECUTION_AGENT_TOOLS_OK instance=execution-isolation",
                "KATA_XIAOO_WORKLOAD_OK instance=execution-isolation",
            ):
                if marker not in output:
                    raise RuntimeError(f"xiaoO workload omitted marker: {marker}")
            results["agent"] = TestResult(
                TestStatus.PASSED,
                "real xiaoO completed in the isolated Guest",
            )

            observed_alerts = self._wait_observation_alerts(
                gateway,
                sb,
                root_pid,
                subscriber,
            )
            self._alerts.assert_independent_from(
                self._run_dir / "data" / "actrail.sqlite"
            )
            results["observation"] = TestResult(
                TestStatus.PASSED,
                "Guest resource and root-lineage I/O observations crossed SB, "
                "gateway, daemon database and alert-proxy subscriber: "
                + ", ".join(sorted(observed_alerts)),
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
        finally:
            if subscriber is not None:
                subscriber.close()
            for name, process in (("workload", workload), ("sb", sb), ("gateway", gateway)):
                if process is None:
                    continue
                try:
                    stopped_process = process.terminate(grace_seconds=3)
                    if (
                        "failure" in results
                        and stopped_process.diagnostic
                    ):
                        results["failure"].message += (
                            f"\n{name} diagnostics:\n"
                            + stopped_process.diagnostic[-4000:]
                        )
                except Exception as error:
                    cleanup_errors.append(f"{name}: {error}")
            if vm is not None:
                try:
                    if "failure" in results:
                        results["failure"].message += (
                            "\nVM diagnostics:\n" + vm.diagnostics()
                        )
                    vm.close()
                except Exception as error:
                    cleanup_errors.append(f"vm: {error}")
            if runtime is not None:
                try:
                    stopped = runtime.stop()
                    if stopped is not None and stopped.returncode != 0:
                        cleanup_errors.append("daemon: " + stopped.output[-1000:])
                except Exception as error:
                    cleanup_errors.append(f"daemon: {error}")
            if proxy is not None:
                try:
                    proxy.terminate()
                except Exception as error:
                    cleanup_errors.append(f"alert-proxy: {error}")
            results["cleanup"] = TestResult(
                TestStatus.FAILED if cleanup_errors else TestStatus.PASSED,
                "; ".join(cleanup_errors) if cleanup_errors else "owned resources removed",
            )
        return TestResult(
            TestStatus.COMPOSITE,
            "real xiaoO execution-isolation observation path",
            results,
        )

    def _prepare_assets(self) -> None:
        assert self._deployment.xiaoo is not None
        self._assets.mkdir(parents=True)
        self._coord.mkdir(mode=0o770)
        os.chown(self._coord, self._config.workload_uid, self._config.workload_gid)
        sources = {
            "xiaoo-real": self._deployment.xiaoo,
            "xiaoo-root": (
                Path(__file__).parent / "assets/named_agent_root.py"
            ),
            "provider_proxy.py": (
                self._config.repo / "tests/support/llm-http-proxy/provider_proxy.py"
            ),
            "workload.sh": Path(__file__).parent / "assets/workload.sh",
            "oom-trigger.sh": Path(__file__).parent / "assets/oom-trigger.sh",
            "oom_trigger.py": Path(__file__).parent / "assets/oom_trigger.py",
        }
        for name, source in sources.items():
            if not source.is_file():
                raise RuntimeError(f"execution-isolation asset is missing: {source}")
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
            self._xiaoo_config(),
            encoding="utf-8",
        )
        (self._assets / "task.txt").write_text(
            "ACTRAIL_EXECUTION_AGENT_READ_INPUT\n",
            encoding="utf-8",
        )
        self._write_sb_config()
        self._write_plugin_assets()
        self._write_manifest(self._assets)

    def _requirements(self) -> KataContainerRequirements:
        builder = KataRequirementsBuilder(
            backend="cloud-hypervisor",
            runtime=self._config.ctr_runtime,
            runtime_config=self._deployment.data_config,
            image=self._deployment.workload_image,
            runner=self._runner,
            pull_policy=self._config.image_pull_policy,
            image_archive=self._config.image_archive,
            runtime_timeout_seconds=self._config.runtime_timeout_seconds,
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            ready_timeout_seconds=self._config.ready_timeout_seconds,
        )
        return builder.build(
            name_prefix="execution-isolation-cloud",
            command=("/bin/sh", "-c", "sleep 600"),
            mounts=(
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
                KataMount(self._coord, "/run/actrail-execution", read_only=False),
                KataMount(Path("/dev/actrail"), "/run/actrail", read_only=True),
            ),
            artifact_directories=(
                self._deployment.workload_bundle,
                self._deployment.guest_bundle,
                self._assets,
            ),
            labels=(("io.actrail.test.case", "execution-isolation"),),
            running_validator=self._validate_vm_ready,
        )

    def _validate_vm_ready(self, vm: KataTestContainer) -> RequirementCheck:
        if not vm.is_running():
            return RequirementCheck.not_ready(
                "execution-isolation Kata task exited before readiness",
                refreshable=True,
            )
        result = vm.exec(
            (
                "/bin/sh",
                "-ec",
                ". /etc/os-release; [ \"${ID:-}\" = openEuler ]; "
                "test -r /sys/kernel/btf/vmlinux; "
                "test -x /opt/actrail-guest/actrail-sb; "
                "test -x /opt/actrail-execution/xiaoo-real; "
                "test -x /opt/actrail-execution/xiaoo-root; "
                "command -v python3 >/dev/null; "
                "/opt/actrail-execution/xiaoo-real --cli run --help "
                "2>&1 | grep -q -- --tools; "
                "remaining=180; while [ $remaining -gt 0 ]; do "
                "[ -S /run/actrail/control.sock ] && exit 0; "
                "remaining=$((remaining - 1)); sleep 0.5; done; exit 72",
            ),
            timeout=self._config.ready_timeout_seconds,
        )
        if result.returncode == 0:
            return RequirementCheck.ready_check()
        return RequirementCheck.not_ready(
            result.diagnostic or "execution-isolation Guest readiness failed",
            refreshable=not any(
                marker in result.diagnostic.lower()
                for marker in ("permission denied", "operation not permitted", "kvm")
            ),
        )

    def _load_plugin(self, runtime: ActrailRuntime) -> None:
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
                "execution-isolation.resource-alert",
            ]
        )
        if "loaded instance=execution-isolation.resource-alert" not in result.output:
            raise RuntimeError("sandbox resource alert plugin did not become active")

    def _write_sb_config(self) -> None:
        self._run_config_init(
            self._config.bin_dir / "actrail-sb",
            (
                "--output",
                str(self._assets / "sb.toml"),
                "--root-process-name",
                "actrail-root",
                "--host-cid",
                "2",
                "--port",
                str(self._config.vsock_port),
                "--instance-lock-path",
                "/run/actrail-sb.lock",
            ),
        )

    def _write_gateway_config(self, socket_path: Path, daemon_port: int) -> None:
        self._run_config_init(
            self._config.bin_dir / "actrail-vsock-gateway",
            (
                "--output",
                str(self._run_dir / "gateway.toml"),
                "--backend",
                "cloud-hypervisor",
                "--socket-path",
                str(socket_path),
                "--daemon-address",
                f"127.0.0.1:{daemon_port}",
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
                f"current release config generator failed: {result.diagnostic}"
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
                "memory_available_threshold_bytes": 18446744073709551615,
                "read_interval_threshold_bytes": 1,
                "write_interval_threshold_bytes": 1,
                "source_state_capacity": 64,
            }
        )
        (self._run_dir / "sandbox-resource-alert.json").write_text(
            json.dumps(document, indent=2) + "\n",
            encoding="utf-8",
        )

    def _workload_environment(self) -> dict[str, str]:
        return {
            "ACTRAIL_XIAOO_INSTANCE": "execution-isolation",
            "ACTRAIL_XIAOO_BIN": "/opt/actrail-execution/xiaoo-root",
            "ACTRAIL_XIAOO_CONFIG": "/opt/actrail-execution/xiaoo.toml",
            "ACTRAIL_XIAOO_PROMPT": (
                "Use the Bash tool calls requested by the provider, then reply "
                "with exactly ACTRAIL_EXECUTION_ISOLATION_XIAOO_OK."
            ),
            "ACTRAIL_XIAOO_RESPONSE_MARKER": (
                "ACTRAIL_EXECUTION_ISOLATION_XIAOO_OK"
            ),
            "ACTRAIL_XIAOO_COORD_DIR": "/run/actrail-execution",
            "ACTRAIL_XIAOO_PROVIDER_SCRIPT": (
                "/opt/actrail-execution/provider_proxy.py"
            ),
            "ACTRAIL_XIAOO_READY_TIMEOUT_SECONDS": str(
                self._config.ready_timeout_seconds
            ),
            "ACTRAIL_EXECUTION_REAL_XIAOO": (
                "/opt/actrail-execution/xiaoo-real"
            ),
            "ACTRAIL_EXECUTION_ROOT_PID_FILE": (
                "/run/actrail-execution/root.pid"
            ),
            "ACTRAIL_EXECUTION_CHILD_RELEASE": (
                "/run/actrail-execution/child.release"
            ),
            "ACTRAIL_EXECUTION_CHILD_TIMEOUT_SECONDS": str(
                self._config.ready_timeout_seconds
            ),
        }

    def _wait_tcp(self, port: int) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError("hand-observation TCP listener did not become ready")

    def _wait_path(
        self,
        path: Path,
        process: ManagedProcess,
        description: str,
    ) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            if path.is_file():
                return
            if process.poll() is not None:
                result = process.wait(timeout=1)
                raise RuntimeError(
                    f"process exited before {description}: {result.diagnostic}"
                )
            time.sleep(0.05)
        raise RuntimeError(f"timed out waiting for {description}")

    def _wait_observation_alerts(
        self,
        gateway: ManagedProcess,
        sb: ManagedProcess,
        root_pid: int,
        subscriber: AlertSubscriberClient,
    ) -> dict[str, SandboxAlertRecord]:
        expected_categories = {
            "sandbox.resource.high_cpu",
            "sandbox.resource.oom_killed",
            "sandbox.resource.oom_risk",
            "sandbox.process.high_read",
            "sandbox.process.high_write",
        }
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self._require_alive(gateway, "gateway")
            self._require_alive(sb, "actrail-sb")
            records = self._select_expected_alerts(root_pid)
            if expected_categories.issubset(records):
                records = {
                    category: records[category]
                    for category in expected_categories
                }
                external = subscriber.wait_for_matching_alerts(
                    max(0.1, deadline - time.monotonic()),
                    {
                        category: (
                            lambda message, record=record: self._matches_delivery(
                                record,
                                message,
                            )
                        )
                        for category, record in records.items()
                    },
                )
                for category, message in external.items():
                    self._assert_delivery(records[category], message)
                return records
            time.sleep(0.05)
        raise RuntimeError(
            "timed out waiting for Guest high-CPU, OOM-killed, OOM-risk, high-read and "
            f"high-write alerts aggregated to named root pid={root_pid}"
        )

    def _trigger_guest_oom(self, vm: KataTestContainer) -> None:
        result = vm.exec(
            ("/bin/sh", "/opt/actrail-execution/oom-trigger.sh"),
            uid=0,
            gid=0,
            timeout=45,
        )
        if result.returncode != 0:
            raise RuntimeError("controlled Guest OOM trigger failed: " + result.diagnostic)
        if "ACTRAIL_EXECUTION_OOM_KILL_OK" not in result.stdout:
            raise RuntimeError("controlled Guest OOM trigger omitted success evidence")

    def _wait_resource_baseline(
        self,
        gateway: ManagedProcess,
        sb: ManagedProcess,
    ) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self._require_alive(gateway, "gateway")
            self._require_alive(sb, "actrail-sb")
            if any(
                record.category == "sandbox.resource.oom_risk"
                for record in self._alerts.records()
            ):
                return
            time.sleep(0.05)
        raise RuntimeError(
            "timed out waiting for the pre-OOM Guest resource baseline"
        )

    def _select_expected_alerts(
        self,
        root_pid: int,
    ) -> dict[str, SandboxAlertRecord]:
        selected: dict[str, SandboxAlertRecord] = {}
        for record in self._alerts.records():
            if record.gateway_id <= 0 or record.sb_id <= 0:
                raise AssertionError(f"invalid sandbox alert source: {record}")
            if record.category.startswith("sandbox.process."):
                if record.process is None or record.process["pid"] != root_pid:
                    continue
            selected.setdefault(record.category, record)
        return selected

    @staticmethod
    def _matches_delivery(
        record: SandboxAlertRecord,
        message: dict[str, object],
    ) -> bool:
        return (
            message.get("cat") == record.category
            and message.get("ts") == record.detected_at_ms
            and message.get("source") == record.delivery_source
            and message.get("extras") == record.extras
        )

    @classmethod
    def _assert_delivery(
        cls,
        record: SandboxAlertRecord,
        message: dict[str, object],
    ) -> None:
        source = message.get("source")
        if not isinstance(source, dict) or "trid" in source:
            raise AssertionError(f"sandbox delivery leaked trace identity: {message}")
        if not cls._matches_delivery(record, message):
            raise AssertionError(
                "sandbox database record and subscriber delivery differ: "
                f"record={record}; delivery={message}"
            )
        expected_severity = (
            "critical"
            if record.category == "sandbox.resource.oom_killed"
            else "warning"
        )
        if message.get("s") != expected_severity:
            raise AssertionError(f"sandbox alert severity changed: {message}")

    def _read_root_pid(self) -> int:
        raw = (self._coord / "root.pid").read_text(encoding="ascii").strip()
        if not raw.isdigit() or int(raw) <= 0:
            raise RuntimeError(f"named Agent root PID is invalid: {raw!r}")
        return int(raw)

    @staticmethod
    def _require_alive(process: ManagedProcess, name: str) -> None:
        time.sleep(0.1)
        if process.poll() is not None:
            result = process.wait(timeout=1)
            raise RuntimeError(f"{name} exited early: {result.diagnostic}")

    @staticmethod
    def _xiaoo_config() -> str:
        return """[llm]
provider = "deepseek"
model = "deepseek-chat"
api_key_env = "ACTRAIL_VIRTUAL_XIAOO_API_KEY"
api_base = "http://127.0.0.1:18098"
max_tokens = 128
context_window = 32768
reasoning_effort = "off"
"""

    @staticmethod
    def _write_manifest(directory: Path) -> None:
        lines = []
        for path in sorted(directory.iterdir()):
            if path.name == "MANIFEST.sha256" or not path.is_file():
                continue
            lines.append(f"{sha256_file(path)}  ./{path.name}\n")
        (directory / "MANIFEST.sha256").write_text(
            "".join(lines),
            encoding="utf-8",
        )
