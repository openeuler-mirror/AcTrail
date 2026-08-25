from __future__ import annotations

import secrets
import socket
import time
import uuid

from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.execution_isolation import (
    ControlledHostOom,
    ControlledHostOomResult,
    SandboxAgentProfile,
    SandboxAgentTiming,
    SandboxAlertPath,
    SandboxAlertThresholds,
)
from tests.v2.common.process import ManagedProcess, SubprocessRunner
from tests.v2.common.runner import TestingContextSingleton

from .config import SandboxOomKilledAlertHostConfig


class SandboxOomKilledAlertHostScenario:
    _CATEGORY = "sandbox.resource.oom_killed"
    _U64_MAX = (1 << 64) - 1

    def __init__(
        self,
        config: SandboxOomKilledAlertHostConfig,
        context: TestingContextSingleton,
    ) -> None:
        self._config = config
        self._context = context
        self._runner = SubprocessRunner()
        self._bin_dir = (
            config.bin_dir
            if config.bin_dir.is_absolute()
            else config.repo / config.bin_dir
        )
        self._run_dir = config.work_dir / f"run-{secrets.token_hex(8)}"

    def run(self) -> TestResult:
        results: dict[str, TestResult] = {}
        alert_path: SandboxAlertPath | None = None
        sandbox_agent: ManagedProcess | None = None
        gateway: ManagedProcess | None = None
        cleanup_errors: list[str] = []
        try:
            self._run_dir.mkdir(parents=True)
            self._context.report_progress(
                "alert_path",
                "starting isolated daemon, alert proxy and public subscriber",
            )
            alert_path = self._create_alert_path()
            alert_path.start(self._config.ready_timeout_seconds)
            self._wait_tcp(alert_path.daemon_port)

            self._context.report_progress(
                "sandbox_transport",
                "starting actrail-sb and native VSOCK gateway",
            )
            profile = SandboxAgentProfile(
                binary=self._bin_dir / "actrail-sb",
                work_dir=self._run_dir,
                runner=self._runner,
                command_timeout_seconds=self._config.command_timeout_seconds,
            )
            timing = profile.refresh_default_config("actrail-root")
            sandbox_agent = self._runner.start(profile.daemon_argv())
            profile.wait_ready(
                sandbox_agent,
                self._config.ready_timeout_seconds,
            )

            self._write_gateway_config(alert_path.daemon_port)
            gateway = self._start_gateway()
            connection = profile.connect(
                host_cid=1,
                port=self._config.vsock_port,
            )

            self._context.report_progress(
                "oom_injection",
                "running one test-owned 32 MiB memory-cgroup OOM",
            )
            oom = ControlledHostOom(
                self._run_dir,
                runner=self._runner,
            ).run_monitored(
                root_discovery_settle_seconds=max(
                    self._config.root_discovery_settle_seconds,
                    timing.minimum_root_discovery_settle_seconds,
                ),
                timeout_seconds=self._config.runtime_timeout_seconds,
            )
            self._context.report_progress(
                "public_delivery",
                "waiting for the matching critical OOM-killed alert",
            )
            delivery = alert_path.wait_for_oom_killed_delivery(
                self._config.ready_timeout_seconds,
                victim_pid=oom.victim_pid,
            )
            self._assert_public_delivery(delivery, oom)

            duplicate_window = (
                max(timing.io_poll_seconds, timing.resource_poll_seconds) * 2
                + 0.25
            )
            alert_path.assert_no_matching_oom_killed_delivery(
                duplicate_window,
                victim_pid=oom.victim_pid,
            )
            self._require_alive(gateway, "native VSOCK gateway")
            self._require_alive(sandbox_agent, "actrail-sb")
            self._context.report_progress(
                "database_correlation",
                "correlating the public delivery with independent storage",
            )
            record = alert_path.assert_persisted_delivery(delivery)
            alert_path.assert_independent_database()

            results["stimulus"] = TestResult(
                TestStatus.PASSED,
                "test-owned 32 MiB memory cgroup killed only the allocator: "
                f"victim_pid={oom.victim_pid}; "
                "kernel_oom_kill="
                f"{oom.kernel_oom_kills_before}->{oom.kernel_oom_kills_after}; "
                "cgroup_oom_kill="
                f"{oom.cgroup_oom_kills_before}->{oom.cgroup_oom_kills_after}",
            )
            results["delivery"] = TestResult(
                TestStatus.PASSED,
                "public subscriber received exactly one critical "
                f"{self._CATEGORY} delivery for victim_pid={oom.victim_pid}",
            )
            results["correlation"] = TestResult(
                TestStatus.PASSED,
                "subscriber delivery matched the committed independent alert "
                f"record alert_id={record.alert_id} and monitored root "
                f"pid={oom.root_marker.pid}",
            )
            results["transport"] = TestResult(
                TestStatus.PASSED,
                "actrail-sb connected through the native VSOCK gateway: "
                + connection.stdout.strip(),
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
        finally:
            for name, process in (
                ("gateway", gateway),
                ("actrail-sb", sandbox_agent),
            ):
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
            if alert_path is not None:
                try:
                    cleanup_errors.extend(alert_path.stop())
                except Exception as error:
                    cleanup_errors.append(f"alert path: {error}")
            results["cleanup"] = TestResult(
                TestStatus.FAILED if cleanup_errors else TestStatus.PASSED,
                (
                    "; ".join(cleanup_errors)
                    if cleanup_errors
                    else "owned resources removed"
                ),
            )
        return TestResult(
            TestStatus.COMPOSITE,
            "focused host-native sandbox OOM-killed alert path",
            results,
        )

    def _create_alert_path(self) -> SandboxAlertPath:
        return SandboxAlertPath(
            repo=self._config.repo,
            bin_dir=self._bin_dir,
            work_dir=self._run_dir,
            context=self._context,
            command_timeout_seconds=self._config.command_timeout_seconds,
            daemon_port=resolve_test_port(
                "SANDBOX_OOM_KILLED_ALERT_HOST_E2E_DAEMON_PORT"
            ),
            subscriber_port=resolve_test_port(
                "SANDBOX_OOM_KILLED_ALERT_HOST_E2E_ALERT_PROXY_PORT"
            ),
            web_port=None,
            categories=(self._CATEGORY,),
            thresholds=SandboxAlertThresholds(
                cpu_usage_basis_points=10_000,
                memory_available_bytes=1,
                read_interval_bytes=self._U64_MAX,
                write_interval_bytes=self._U64_MAX,
            ),
        )

    def _write_gateway_config(self, daemon_port: int) -> None:
        result = self._runner.run(
            (
                str(self._bin_dir / "actrail-vsock-gateway"),
                "init",
                "--output",
                str(self._run_dir / "gateway.toml"),
                "--backend",
                "native",
                "--cid",
                "1",
                "--port",
                str(self._config.vsock_port),
                "--daemon-address",
                f"127.0.0.1:{daemon_port}",
            ),
            timeout=self._config.command_timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "current release gateway config generator failed: "
                + result.diagnostic
            )

    def _start_gateway(self) -> ManagedProcess:
        gateway = self._runner.start(
            (
                str(self._bin_dir / "actrail-vsock-gateway"),
                "--config",
                str(self._run_dir / "gateway.toml"),
            )
        )
        try:
            gateway.wait_for_output(
                "gateway ready gateway_id=",
                timeout=self._config.ready_timeout_seconds,
            )
        except Exception:
            gateway.terminate(grace_seconds=1)
            raise
        return gateway

    @classmethod
    def _assert_public_delivery(
        cls,
        message: dict[str, object],
        oom: ControlledHostOomResult,
    ) -> None:
        if message.get("cat") != cls._CATEGORY:
            raise AssertionError(f"OOM delivery category changed: {message}")
        if message.get("s") != "critical":
            raise AssertionError(f"OOM delivery severity changed: {message}")
        detected_at_ms = message.get("ts")
        if (
            not isinstance(detected_at_ms, int)
            or detected_at_ms < oom.released_at_ms
        ):
            raise AssertionError(
                f"OOM delivery timestamp predates injection: {message}"
            )
        source = message.get("source")
        if not isinstance(source, dict) or "trid" in source:
            raise AssertionError(f"OOM delivery leaked trace identity: {message}")
        sandbox = source.get("sandbox")
        if not isinstance(sandbox, dict):
            raise AssertionError(f"OOM delivery omitted sandbox source: {message}")
        for field in ("gateway_id", "sb_id"):
            value = sandbox.get(field)
            if not isinstance(value, int) or value <= 0:
                raise AssertionError(f"OOM delivery has invalid {field}: {message}")
        boot_id = sandbox.get("boot_id")
        try:
            uuid.UUID(str(boot_id))
        except (ValueError, AttributeError) as error:
            raise AssertionError(
                f"OOM delivery has invalid boot_id: {message}"
            ) from error
        if sandbox.get("process") != oom.root_marker.as_process():
            raise AssertionError(f"OOM delivery root marker changed: {message}")
        extras = message.get("extras")
        if not isinstance(extras, dict):
            raise AssertionError(f"OOM delivery omitted extras: {message}")
        expected = {
            "victim_pid": oom.victim_pid,
            "victim_comm": "python3",
            "attribution": "monitored",
        }
        changed = {
            field: extras.get(field)
            for field, value in expected.items()
            if extras.get(field) != value
        }
        if changed:
            raise AssertionError(
                "OOM delivery victim attribution changed: "
                f"{changed}; delivery={message}"
            )

    def _wait_tcp(self, port: int) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError("hand-observation listener did not become ready")

    @staticmethod
    def _require_alive(process: ManagedProcess, name: str) -> None:
        time.sleep(0.1)
        if process.poll() is not None:
            result = process.wait(timeout=1)
            raise RuntimeError(f"{name} exited early: {result.diagnostic}")
