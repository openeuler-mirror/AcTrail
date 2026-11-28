"""Own the Guest observation path and verify resource alerts through scoped steps."""
from __future__ import annotations

from contextlib import ExitStack, contextmanager
import secrets
import time

from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.alert_proxy import (
    AlertProxyTestProfile,
    AlertSubscriberClient,
)
from tests.e2e.common.core.loopback_port import resolve_test_port
from tests.e2e.common.kata_runtime import (
    CtrCapabilities,
    DeploymentArtifacts,
    GuestConsole,
    KataTestContainer,
)
from tests.e2e.common.process import ManagedProcess, SubprocessRunner
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.sandbox_alert_database import SandboxAlertDatabase

from ..backends import ExecutionIsolationBackend
from ..config import KataExecutionIsolationConfig
from .setup import ExecutionIsolationSetup
from .system_observer import GuestSystemSandboxObserver
from .transport import CoordinationDirectory
from .verifier import SandboxAlertVerifier

from ..constants import CASE_CONSTANTS


class KataExecutionIsolationScenario:
    _ALERT_CATEGORIES = (
        "sandbox.resource.high_cpu",
        "sandbox.resource.oom_killed",
        "sandbox.resource.oom_risk",
        "sandbox.process.high_read",
        "sandbox.process.high_write",
    )

    def __init__(
        self,
        config: KataExecutionIsolationConfig,
        deployment: DeploymentArtifacts,
    ) -> None:
        self._config = config
        self._deployment = deployment
        self._runner = SubprocessRunner()
        self._run_token = secrets.token_hex(8)
        self._run_dir = config.work_dir / f"run-{self._run_token}"
        self._assets = self._run_dir / "assets"
        self._coordination = self._run_dir / "coord"
        self._alerts = SandboxAlertDatabase(
            self._run_dir / "data" / "sandbox-alerts.sqlite"
        )
        self._inventory = ExecutionIsolationBackend(config.BACKEND).socket_inventory(
            config.vm_root,
        )
        guest = GuestConsole(self._runner)
        self._observer = GuestSystemSandboxObserver(guest, config)
        self._setup = ExecutionIsolationSetup(
            config,
            deployment,
            self._runner,
            self._run_dir,
            self._assets,
            self._coordination,
        )
        self._verifier = SandboxAlertVerifier(
            config,
            self._alerts,
            self._coordination,
            guest,
        )

    def run(self, case) -> None:
        # 资源按创建顺序登记，后续任何步骤失败均按逆序释放。
        with ExitStack() as resources:
            with case.step("Start AcTrail on the host"):
                self._setup.prepare_assets()
                daemon_port = resolve_test_port(f"{self._config.ENVIRONMENT_PREFIX}DAEMON_PORT")
                subscriber_port = resolve_test_port(f"{self._config.ENVIRONMENT_PREFIX}ALERT_PROXY_PORT")
                proxy = AlertProxyTestProfile.create(
                    self._run_dir, self._config.bin_dir / "actraild-alert-proxy",
                    subscriber_port, secrets.token_urlsafe(24),
                )
                resources.enter_context(self._cleanup_scope("alert-proxy", proxy.terminate))
                proxy.write_forwarding_config(enabled=True, categories=list(self._ALERT_CATEGORIES))
                runtime = resources.enter_context(self._create_runtime(proxy, daemon_port))
                runtime.prepare()
                proxy.require_running()
                subscriber = self._start_subscriber(proxy)
                resources.enter_context(self._cleanup_scope("subscriber", subscriber.close))
                self._setup.load_plugin(runtime)
                self._verifier.wait_tcp(daemon_port)
            with case.step("Start the Kata VM"):
                # 必须在 VM 启动前拍摄 socket 清单，才能识别此次新建的实例。
                before = self._inventory.snapshot() if self._inventory is not None else None
                vm = resources.enter_context(KataTestContainer(
                    self._setup.requirements(), self._runner, CtrCapabilities.detect(self._runner),
                ))
                gateway = self._start_vm_gateway_and_connect(vm, daemon_port, before, case)
                resources.enter_context(self._process_scope("gateway", gateway))
            with case.step("Prepare the guest workload files"):
                self._setup.stage_assets(vm)
                coordination = self._setup.coordination(vm)
            with self._process_scope("workload", self._start_workload(vm)) as workload:
                self._reach_pre_oom_checkpoint(vm, gateway, workload, coordination, case)
                with case.step("Wait for Agent discovery in the guest"):
                    self._verifier.wait_path(coordination.file("root.pid"), workload, "named Agent root discovery")
                    root_pid = self._verifier.read_root_pid(vm, coordination)
                    time.sleep(self._config.root_discovery_settle_seconds)
                    coordination.file("child.release").touch()
                    self._complete_workload(workload)
            with case.step("Check the guest observation alerts") as step:
                observed_alerts = self._verifier.wait_observation_alerts(gateway, root_pid, subscriber)
                self._alerts.assert_independent_from(self._run_dir / "data" / "actrail.sqlite")
                step.message = "Guest resource and root-lineage observations: " + ", ".join(sorted(observed_alerts))

    @contextmanager
    def _cleanup_scope(self, name, cleanup):
        """清理失败附到原异常，保留业务失败和所有退出错误。"""
        error = None
        try:
            yield
        except BaseException as caught:
            error = caught
            raise
        finally:
            try:
                cleanup()
            except Exception as cleanup_error:
                if error is None:
                    raise
                error.add_note(f"{name} cleanup failed: {cleanup_error}")

    @contextmanager
    def _process_scope(self, name: str, process: ManagedProcess):
        error = None
        try:
            yield process
        except BaseException as caught:
            error = caught
            raise
        finally:
            try:
                stopped = process.terminate(grace_seconds=CASE_CONSTANTS.stop_processes_grace_seconds)
                if error is not None and stopped.diagnostic:
                    error.add_note(f"{name} diagnostics: {stopped.diagnostic[-4000:]}")
            except Exception as cleanup_error:
                if error is None:
                    raise
                error.add_note(f"{name} cleanup failed: {cleanup_error}")

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
        before,
        case,
    ) -> ManagedProcess:
        with case.step("Check that the guest observer is ready"):
            self._observer.require_ready_and_unconnected(vm)

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
            with case.step("Record the gateway connection details") as step:
                step.message = gateway_evidence
            with case.step("Connect the guest observer"):
                self._observer.connect(vm)
        except BaseException as error:
            try:
                gateway.terminate(grace_seconds=CASE_CONSTANTS.start_vm_gateway_and_connect_grace_seconds)
            except Exception as cleanup_error:
                error.add_note(
                    "gateway cleanup after startup failure also failed: "
                    f"{cleanup_error}"
                )
            raise
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
            TestingContextSingleton.current().output,
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
        try:
            subscriber.connect(self._config.ready_timeout_seconds)
            subscriber.subscribe(
                identity.SUBSCRIPTION,
                list(self._ALERT_CATEGORIES),
                [],
                self._config.ready_timeout_seconds,
            )
            subscriber.wait_for_heartbeat(self._config.ready_timeout_seconds)
        except BaseException:
            subscriber.close()
            raise
        return subscriber

    def _start_workload(self, vm: KataTestContainer) -> ManagedProcess:
        return vm.start_exec(
            ("/bin/sh", "/opt/actrail-execution/workload.sh"),
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            environment=self._setup.workload_environment(),
        )

    def _reach_pre_oom_checkpoint(self, vm, gateway, workload, coordination, case) -> None:
        with case.step("Wait for the XiaoO provider"):
            self._verifier.wait_path(coordination.file("provider.ready"), workload, "real xiaoO provider readiness")
        with case.step("Wait for baseline resource observations"):
            self._verifier.wait_resource_baseline(gateway)
        with case.step("Trigger an out-of-memory event in the guest"):
            self._verifier.trigger_guest_oom(vm)
            coordination.file("release").touch()

    def _complete_workload(self, workload: ManagedProcess) -> None:
        result = workload.wait(
            timeout=self._config.runtime_timeout_seconds,
            terminate_grace_seconds=CASE_CONSTANTS.complete_workload_terminate_grace_seconds,
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



