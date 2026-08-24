from __future__ import annotations

import secrets
import time

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.alert_proxy import (
    AlertProxyTestProfile,
    AlertSubscriberClient,
)
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.kata_runtime import (
    CtrCapabilities,
    DeploymentArtifacts,
    KataTestContainer,
)
from tests.v2.common.process import ManagedProcess, SubprocessRunner
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.sandbox_alert_database import SandboxAlertDatabase

from ..cloud_hypervisor import CloudHypervisorSocketInventory
from ..config import CloudHypervisorExecutionIsolationConfig
from ..identity import CloudHypervisorScenarioIdentity
from .setup import CloudHypervisorScenarioSetup
from .verifier import CloudHypervisorAlertVerifier


class CloudHypervisorExecutionIsolationScenario:
    _SB_CONTROL_SOCKET = "/run/actrail-sb-control.sock"
    _ALERT_CATEGORIES = (
        "sandbox.resource.high_cpu",
        "sandbox.resource.oom_killed",
        "sandbox.resource.oom_risk",
        "sandbox.process.high_read",
        "sandbox.process.high_write",
    )

    def __init__(
        self,
        config: CloudHypervisorExecutionIsolationConfig,
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
        self._coordination = self._run_dir / "coord"
        self._alerts = SandboxAlertDatabase(
            self._run_dir / "data" / "sandbox-alerts.sqlite"
        )
        self._inventory = CloudHypervisorSocketInventory(config.vm_root)
        self._setup = CloudHypervisorScenarioSetup(
            config,
            deployment,
            self._runner,
            self._run_dir,
            self._assets,
            self._coordination,
        )
        self._verifier = CloudHypervisorAlertVerifier(
            config,
            self._alerts,
            self._coordination,
        )

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
            self._setup.prepare_assets()
            daemon_port = resolve_test_port(
                "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_DAEMON_PORT"
            )
            subscriber_port = resolve_test_port(
                "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_ALERT_PROXY_PORT"
            )
            proxy = AlertProxyTestProfile.create(
                self._run_dir,
                self._config.bin_dir / "actraild-alert-proxy",
                subscriber_port,
                secrets.token_urlsafe(24),
            )
            proxy.write_forwarding_config(
                enabled=True,
                categories=list(self._ALERT_CATEGORIES),
            )
            runtime = self._create_runtime(proxy, daemon_port)
            self._context.report_progress(
                "daemon",
                "starting isolated hand-observation listener and plugin",
            )
            runtime.prepare()
            proxy.require_running()
            subscriber = self._start_subscriber(proxy)
            self._setup.load_plugin(runtime)
            self._verifier.wait_tcp(daemon_port)
            results["daemon"] = TestResult(
                TestStatus.PASSED,
                "hand-observation listener and sandbox plugin are active",
            )

            before = self._inventory.snapshot()
            vm = KataTestContainer(
                self._setup.requirements(),
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
            self._setup.write_gateway_config(gateway_socket, daemon_port)
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
            self._verifier.require_alive(gateway, "gateway")
            results["gateway"] = TestResult(
                TestStatus.PASSED,
                f"gateway owns {gateway_socket}",
            )

            sb = self._start_sb_daemon(vm)
            self._verifier.require_alive(sb, "actrail-sb daemon")
            self._wait_sb_daemon_ready(vm, sb)
            self._connect_sb(vm)
            results["sb"] = TestResult(
                TestStatus.PASSED,
                "actrail-sb daemon accepted the runtime VSOCK endpoint",
            )

            workload = self._start_workload(vm)
            self._verifier.wait_path(
                self._coordination / "provider.ready",
                workload,
                "real xiaoO provider readiness",
            )
            self._verifier.wait_resource_baseline(gateway, sb)
            self._verifier.trigger_guest_oom(vm)
            (self._coordination / "release").touch()
            self._verifier.wait_path(
                self._coordination / "root.pid",
                workload,
                "named Agent root discovery",
            )
            root_pid = self._verifier.read_root_pid()
            time.sleep(self._config.root_discovery_settle_seconds)
            (self._coordination / "child.release").touch()
            self._complete_workload(workload)
            workload = None
            results["agent"] = TestResult(
                TestStatus.PASSED,
                "real xiaoO completed in the isolated Guest",
            )

            observed_alerts = self._verifier.wait_observation_alerts(
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
            self._stop_processes(
                results,
                cleanup_errors,
                (("workload", workload), ("sb", sb), ("gateway", gateway)),
            )
            self._stop_vm(vm, results, cleanup_errors)
            self._stop_runtime(runtime, cleanup_errors)
            if proxy is not None:
                try:
                    proxy.terminate()
                except Exception as error:
                    cleanup_errors.append(f"alert-proxy: {error}")
            results["cleanup"] = TestResult(
                TestStatus.FAILED if cleanup_errors else TestStatus.PASSED,
                "; ".join(cleanup_errors)
                if cleanup_errors
                else "owned resources removed",
            )
        return TestResult(
            TestStatus.COMPOSITE,
            "Cloud Hypervisor execution-isolation observation path",
            results,
        )

    def _create_runtime(
        self,
        proxy: AlertProxyTestProfile,
        daemon_port: int,
    ) -> ActrailRuntime:
        return ActrailRuntime.isolated(
            self._config.repo,
            self._config.bin_dir,
            self._config.command_timeout_seconds,
            self._context.output,
            self._run_dir,
            hand_observation_listen_addr=f"127.0.0.1:{daemon_port}",
            sandbox_alerts_database=(
                self._run_dir / "data" / "sandbox-alerts.sqlite"
            ),
            alert_forwarding=proxy.runtime_paths,
            clean_control_state=False,
        )

    def _start_subscriber(
        self,
        proxy: AlertProxyTestProfile,
    ) -> AlertSubscriberClient:
        identity = CloudHypervisorScenarioIdentity
        subscriber = AlertSubscriberClient(
            proxy.subscriber_address,
            proxy.token,
            identity.subscriber_client(self._run_token),
            self._run_dir / "alert-subscriber.jsonl",
        )
        subscriber.connect(self._config.ready_timeout_seconds)
        subscriber.subscribe(
            identity.SUBSCRIPTION,
            list(self._ALERT_CATEGORIES),
            [],
            self._config.ready_timeout_seconds,
        )
        subscriber.wait_for_heartbeat(self._config.ready_timeout_seconds)
        return subscriber

    def _start_workload(self, vm: KataTestContainer) -> ManagedProcess:
        return vm.start_exec(
            ("/bin/sh", "/opt/actrail-execution/workload.sh"),
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            environment=self._setup.workload_environment(),
        )

    @staticmethod
    def _start_sb_daemon(vm: KataTestContainer) -> ManagedProcess:
        return vm.start_exec(
            (
                "/opt/actrail-guest/actrail-sb",
                "daemon",
                "--config",
                "/opt/actrail-execution/sb.toml",
            ),
            uid=0,
            gid=0,
            environment={"LD_LIBRARY_PATH": "/opt/actrail-guest/lib"},
        )

    def _wait_sb_daemon_ready(
        self,
        vm: KataTestContainer,
        daemon: ManagedProcess,
    ) -> None:
        ready = vm.exec(
            (
                "/bin/sh",
                "-ec",
                f"remaining={self._config.ready_timeout_seconds * 2}; "
                "while [ $remaining -gt 0 ]; do "
                f"[ -S {self._SB_CONTROL_SOCKET} ] && exit 0; "
                "remaining=$((remaining - 1)); sleep 0.5; "
                "done; exit 72",
            ),
            uid=0,
            gid=0,
            timeout=self._config.ready_timeout_seconds + 1,
        )
        if ready.returncode != 0:
            self._verifier.require_alive(daemon, "actrail-sb daemon")
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    "actrail-sb daemon control socket did not become ready: "
                    + ready.diagnostic
                )
            )

    def _connect_sb(self, vm: KataTestContainer) -> None:
        connected = vm.exec(
            (
                "/opt/actrail-guest/actrail-sb",
                "connect",
                "--control-socket",
                self._SB_CONTROL_SOCKET,
                "--host-cid",
                str(self._config.vsock_host_cid),
                "--port",
                str(self._config.vsock_port),
            ),
            uid=0,
            gid=0,
            environment={"LD_LIBRARY_PATH": "/opt/actrail-guest/lib"},
            timeout=self._config.ready_timeout_seconds,
        )
        if connected.returncode != 0:
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    "actrail-sb runtime VSOCK connection failed: "
                    + connected.diagnostic
                )
            )
        if "actrail-sb connected sb_id=" not in connected.diagnostic:
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    "actrail-sb connect omitted the successful handshake marker"
                )
            )

    def _complete_workload(self, workload: ManagedProcess) -> None:
        result = workload.wait(
            timeout=self._config.runtime_timeout_seconds,
            terminate_grace_seconds=5,
        )
        if result.returncode != 0:
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    "real xiaoO workload failed: " + result.diagnostic
                )
            )
        output = result.stdout + result.stderr
        for marker in CloudHypervisorScenarioIdentity.workload_markers():
            if marker not in output:
                raise RuntimeError(
                    CloudHypervisorScenarioIdentity.failure(
                        f"xiaoO workload omitted marker: {marker}"
                    )
                )

    @staticmethod
    def _stop_processes(
        results: dict[str, TestResult],
        cleanup_errors: list[str],
        processes: tuple[tuple[str, ManagedProcess | None], ...],
    ) -> None:
        for name, process in processes:
            if process is None:
                continue
            try:
                stopped = process.terminate(grace_seconds=3)
                if "failure" in results and stopped.diagnostic:
                    results["failure"].message += (
                        f"\n{name} diagnostics:\n" + stopped.diagnostic[-4000:]
                    )
            except Exception as error:
                cleanup_errors.append(f"{name}: {error}")

    @staticmethod
    def _stop_vm(
        vm: KataTestContainer | None,
        results: dict[str, TestResult],
        cleanup_errors: list[str],
    ) -> None:
        if vm is None:
            return
        try:
            if "failure" in results:
                results["failure"].message += "\nVM diagnostics:\n" + vm.diagnostics()
            vm.close()
        except Exception as error:
            cleanup_errors.append(f"vm: {error}")

    @staticmethod
    def _stop_runtime(
        runtime: ActrailRuntime | None,
        cleanup_errors: list[str],
    ) -> None:
        if runtime is None:
            return
        try:
            stopped = runtime.stop()
            if stopped is not None and stopped.returncode != 0:
                cleanup_errors.append("daemon: " + stopped.output[-1000:])
        except Exception as error:
            cleanup_errors.append(f"daemon: {error}")
