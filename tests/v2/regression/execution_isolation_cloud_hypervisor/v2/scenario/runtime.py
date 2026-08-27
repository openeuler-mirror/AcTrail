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
    GuestConsole,
    KataTestContainer,
)
from tests.v2.common.process import ManagedProcess, SubprocessRunner
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.sandbox_alert_database import SandboxAlertDatabase

from ..cloud_hypervisor import (
    CloudHypervisorSocketInventory,
    FirecrackerSocketInventory,
    HybridVsockSocketInventory,
)
from ..config import CloudHypervisorExecutionIsolationConfig
from .setup import CloudHypervisorScenarioSetup
from .system_observer import GuestSystemSandboxObserver
from .transport import CoordinationDirectory
from .verifier import CloudHypervisorAlertVerifier


class CloudHypervisorExecutionIsolationScenario:
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
        self._inventory: HybridVsockSocketInventory | None
        if config.BACKEND == "cloud-hypervisor":
            self._inventory = CloudHypervisorSocketInventory(config.vm_root)
        elif config.BACKEND == "firecracker":
            self._inventory = FirecrackerSocketInventory(config.vm_root)
        else:
            self._inventory = None
        guest = GuestConsole(self._runner)
        self._observer = GuestSystemSandboxObserver(guest, config)
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
            guest,
        )

    def run(self) -> TestResult:
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        proxy: AlertProxyTestProfile | None = None
        subscriber: AlertSubscriberClient | None = None
        vm: KataTestContainer | None = None
        gateway: ManagedProcess | None = None
        workload: ManagedProcess | None = None
        cleanup_errors: list[str] = []
        try:
            self._setup.prepare_assets()
            daemon_port = resolve_test_port(
                f"{self._config.ENVIRONMENT_PREFIX}DAEMON_PORT"
            )
            subscriber_port = resolve_test_port(
                f"{self._config.ENVIRONMENT_PREFIX}ALERT_PROXY_PORT"
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

            vm = KataTestContainer(
                self._setup.requirements(),
                self._runner,
                CtrCapabilities.detect(self._runner),
            )
            self._context.report_progress(
                "vm",
                f"starting one test-owned {self._config.IDENTITY.DISPLAY} Kata VM",
            )
            gateway = self._start_vm_gateway_and_connect(
                vm,
                daemon_port,
                results,
            )
            self._context.report_progress(
                "assets",
                "staging the execution-isolation workload in the Guest",
            )
            self._setup.stage_assets(vm)
            coordination = self._setup.coordination(vm)

            workload = self._start_workload(vm)
            self._reach_pre_oom_checkpoint(
                vm,
                gateway,
                workload,
                coordination,
            )
            self._context.report_progress(
                "agent",
                "running real xiaoO and discovering its named root process",
            )
            self._verifier.wait_path(
                coordination.file("root.pid"),
                workload,
                "named Agent root discovery",
            )
            root_pid = self._verifier.read_root_pid(vm, coordination)
            time.sleep(self._config.root_discovery_settle_seconds)
            coordination.file("child.release").touch()
            self._complete_workload(workload)
            workload = None
            results["agent"] = TestResult(
                TestStatus.PASSED,
                "real xiaoO completed in the isolated Guest",
            )

            self._context.report_progress(
                "alerts",
                "verifying Guest resource and root-lineage observations",
            )
            observed_alerts = self._verifier.wait_observation_alerts(
                gateway,
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
                (("workload", workload), ("gateway", gateway)),
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
            f"{self._config.IDENTITY.DISPLAY} observation path",
            results,
        )

    def _start_gateway(self) -> ManagedProcess:
        return self._runner.start(
            (
                str(self._deployment.host_bundle / "actrail-vsock-gateway"),
                "--config",
                str(self._run_dir / "gateway.toml"),
            )
        )

    def _start_vm_gateway_and_connect(
        self,
        vm: KataTestContainer,
        daemon_port: int,
        results: dict[str, TestResult],
    ) -> ManagedProcess:
        before = self._inventory.snapshot() if self._inventory is not None else None
        vm.start()
        self._observer.require_ready_and_unconnected(vm)
        results["observer-ready"] = TestResult(
            TestStatus.PASSED,
            "Guest system actrail-sb daemon is ready and unconnected",
        )

        if self._inventory is None:
            self._setup.write_gateway_config(None, daemon_port)
            listener = f"native AF_VSOCK port {self._config.vsock_port}"
            base_socket = None
        else:
            assert before is not None
            base_socket = self._inventory.wait_new_base_socket(
                before,
                self._config.ready_timeout_seconds,
            )
            gateway_socket = self._inventory.gateway_socket(
                base_socket,
                self._config.vsock_port,
            )
            self._setup.write_gateway_config(gateway_socket, daemon_port)
            listener = str(
                self._inventory.listener_socket(
                    base_socket,
                    self._config.vsock_port,
                )
            )

        gateway = self._start_gateway()
        try:
            self._verifier.require_alive(gateway, "gateway")
            if self._inventory is not None:
                assert base_socket is not None
                self._inventory.wait_listener_socket(
                    base_socket,
                    self._config.vsock_port,
                    self._config.ready_timeout_seconds,
                )
                gateway_evidence = f"gateway listener is ready on {listener}"
            else:
                gateway.wait_for_output(
                    "gateway ready gateway_id=",
                    timeout=self._config.ready_timeout_seconds,
                )
                gateway_evidence = (
                    f"gateway reported ready on {listener} before the explicit "
                    "Guest observer connection"
                )
            results["gateway"] = TestResult(
                TestStatus.PASSED,
                gateway_evidence,
            )
            self._observer.connect(vm)
        except BaseException as error:
            try:
                gateway.terminate(grace_seconds=3)
            except Exception as cleanup_error:
                error.add_note(
                    "gateway cleanup after startup failure also failed: "
                    f"{cleanup_error}"
                )
            raise
        results["observer-connect"] = TestResult(
            TestStatus.PASSED,
            "case explicitly connected the Guest system observer after gateway "
            "readiness",
        )
        return gateway

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
        identity = self._config.IDENTITY
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

    def _reach_pre_oom_checkpoint(
        self,
        vm: KataTestContainer,
        gateway: ManagedProcess,
        workload: ManagedProcess,
        coordination: CoordinationDirectory,
    ) -> None:
        self._context.report_progress(
            "provider",
            "waiting for the real xiaoO local provider in the Guest",
        )
        self._verifier.wait_path(
            coordination.file("provider.ready"),
            workload,
            "real xiaoO provider readiness",
        )
        self._context.report_progress(
            "resource-baseline",
            "waiting for the pre-OOM Guest resource observation",
        )
        self._verifier.wait_resource_baseline(gateway)
        self._context.report_progress(
            "guest-oom",
            "triggering the controlled OOM in the Guest root cgroup namespace",
        )
        self._verifier.trigger_guest_oom(vm)
        coordination.file("release").touch()

    def _complete_workload(self, workload: ManagedProcess) -> None:
        result = workload.wait(
            timeout=self._config.runtime_timeout_seconds,
            terminate_grace_seconds=5,
        )
        if result.returncode != 0:
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    "real xiaoO workload failed: " + result.diagnostic
                )
            )
        output = result.stdout + result.stderr
        for marker in self._config.IDENTITY.workload_markers():
            if marker not in output:
                raise RuntimeError(
                    self._config.IDENTITY.failure(
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
