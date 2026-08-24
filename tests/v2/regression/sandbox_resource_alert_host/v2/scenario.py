from __future__ import annotations

import os
import secrets
import shutil
import socket
import time
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.agent_selection import AgentSelection
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.core.loopback_port import resolve_test_port
from tests.v2.common.execution_isolation import (
    SandboxAgentProfile,
    SandboxAgentTiming,
    SandboxAlertPath,
    SandboxAlertThresholds,
)
from tests.v2.common.process import CommandResult, ManagedProcess, SubprocessRunner
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.sandbox_alert_database import SandboxAlertRecord

from .config import SandboxResourceAlertHostConfig


@dataclass
class _OwnedProcesses:
    gateway: ManagedProcess | None = None
    sandbox_agent: ManagedProcess | None = None
    workload: ManagedProcess | None = None


@dataclass(frozen=True)
class _ScenarioOutcome:
    records: dict[str, SandboxAlertRecord]
    initial_connection: str
    reconnection: str


class SandboxResourceAlertHostScenario:
    _READ_THRESHOLD_BYTES = 786_432
    _WRITE_THRESHOLD_BYTES = 2_097_152
    _CATEGORIES = (
        "sandbox.resource.oom_risk",
        "sandbox.process.high_read",
        "sandbox.process.high_write",
    )

    def __init__(
        self,
        config: SandboxResourceAlertHostConfig,
        context: TestingContextSingleton,
        agent: AgentSelection,
    ) -> None:
        self._config = config
        self._context = context
        self._agent = agent
        self._runner = SubprocessRunner()
        self._run_dir = config.work_dir / f"run-{secrets.token_hex(8)}"
        self._assets = self._run_dir / "assets"
        self._coord = self._run_dir / "coord"
        self._provider_port = 0

    def run(self) -> TestResult:
        results: dict[str, TestResult] = {}
        alert_path: SandboxAlertPath | None = None
        processes = _OwnedProcesses()
        cleanup_errors: list[str] = []
        try:
            alert_path = self._start_alert_path()
            outcome = self._execute_path(alert_path, processes)
            results["observation"] = TestResult(
                TestStatus.PASSED,
                "host resource and named-root I/O observations reached the "
                "independent database and subscriber: "
                + ", ".join(sorted(outcome.records)),
            )
            results["agent"] = TestResult(
                TestStatus.PASSED,
                "real xiaoO completed host file read and write tool calls",
            )
            results["control"] = TestResult(
                TestStatus.PASSED,
                "daemon remained idle without persistence before connection; "
                "CLI connection and reconnection succeeded without replay: "
                f"initial={outcome.initial_connection}; "
                f"reconnect={outcome.reconnection}",
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
        finally:
            for name, process in (
                ("workload", processes.workload),
                ("sb", processes.sandbox_agent),
                ("gateway", processes.gateway),
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
            "host-native sandbox resource alert path",
            results,
        )

    def _start_alert_path(self) -> SandboxAlertPath:
        provider_port = resolve_test_port(
            "SANDBOX_RESOURCE_ALERT_HOST_E2E_PROVIDER_PORT"
        )
        self._provider_port = provider_port
        self._prepare_assets(provider_port)
        daemon_port = resolve_test_port(
            "SANDBOX_RESOURCE_ALERT_HOST_E2E_DAEMON_PORT"
        )
        subscriber_port = resolve_test_port(
            "SANDBOX_RESOURCE_ALERT_HOST_E2E_ALERT_PROXY_PORT"
        )
        alert_path = SandboxAlertPath(
            repo=self._config.repo,
            bin_dir=self._config.bin_dir,
            work_dir=self._run_dir,
            context=self._context,
            command_timeout_seconds=self._config.command_timeout_seconds,
            daemon_port=daemon_port,
            subscriber_port=subscriber_port,
            categories=self._CATEGORIES,
            thresholds=SandboxAlertThresholds(
                cpu_usage_basis_points=10_000,
                memory_available_bytes=18_446_744_073_709_551_615,
                read_interval_bytes=self._READ_THRESHOLD_BYTES,
                write_interval_bytes=self._WRITE_THRESHOLD_BYTES,
            ),
        )
        alert_path.start(self._config.ready_timeout_seconds)
        self._wait_tcp(daemon_port)
        return alert_path

    def _execute_path(
        self,
        alert_path: SandboxAlertPath,
        processes: _OwnedProcesses,
    ) -> _ScenarioOutcome:
        profile = SandboxAgentProfile(
            binary=self._config.bin_dir / "actrail-sb",
            work_dir=self._run_dir,
            runner=self._runner,
            command_timeout_seconds=self._config.command_timeout_seconds,
        )
        timing = profile.refresh_default_config("actrail-root")
        processes.sandbox_agent = self._runner.start(profile.daemon_argv())
        profile.wait_ready(
            processes.sandbox_agent,
            self._config.ready_timeout_seconds,
        )
        self._verify_disconnected_no_storage(
            alert_path,
            processes.sandbox_agent,
            timing.disconnected_observation_window_seconds,
        )

        self._write_gateway_config(alert_path.daemon_port)
        processes.gateway = self._start_gateway()
        initial = profile.connect(host_cid=1, port=self._config.vsock_port)
        records = self._run_alert_workload(
            alert_path,
            processes,
        )
        reconnect = self._verify_disconnect_reconnect(
            alert_path,
            profile,
            timing,
            processes,
        )
        return _ScenarioOutcome(
            records,
            initial.stdout.strip(),
            reconnect.stdout.strip(),
        )

    def _run_alert_workload(
        self,
        alert_path: SandboxAlertPath,
        processes: _OwnedProcesses,
    ) -> dict[str, SandboxAlertRecord]:
        gateway = self._require_owned(processes.gateway, "gateway")
        sandbox_agent = self._require_owned(
            processes.sandbox_agent,
            "actrail-sb",
        )
        processes.workload = self._runner.start(
            ("/bin/sh", str(self._assets / "workload.sh")),
            environment=self._workload_environment(),
        )
        workload = processes.workload
        self._wait_path(
            self._coord / "provider.ready",
            workload,
            "real xiaoO provider readiness",
        )
        self._wait_memory_baseline(alert_path, gateway, sandbox_agent)

        (self._coord / "workload.release").touch()
        self._wait_path(
            self._coord / "root.pid",
            workload,
            "named Agent root discovery",
        )
        root_marker = self._read_root_marker()
        time.sleep(self._config.root_discovery_settle_seconds)
        child_release_ms = int(time.time() * 1000)
        (self._coord / "child.release").touch()
        completed = workload.wait(timeout=self._config.runtime_timeout_seconds)
        processes.workload = None
        if completed.returncode != 0:
            raise RuntimeError(
                "real xiaoO workload failed: " + completed.diagnostic
            )
        if "ACTRAIL_HOST_XIAOO_WORKLOAD_OK" not in completed.stdout:
            raise RuntimeError("real xiaoO workload omitted completion evidence")

        records = self._wait_all_alerts(
            alert_path,
            gateway,
            sandbox_agent,
            root_marker,
            child_release_ms,
        )
        deliveries = alert_path.matching_deliveries(
            records,
            self._config.ready_timeout_seconds,
        )
        for category, message in deliveries.items():
            alert_path.assert_delivery(records[category], message)
        alert_path.assert_independent_database()
        return records

    def _verify_disconnect_reconnect(
        self,
        alert_path: SandboxAlertPath,
        profile: SandboxAgentProfile,
        timing: SandboxAgentTiming,
        processes: _OwnedProcesses,
    ) -> CommandResult:
        gateway = self._require_owned(processes.gateway, "gateway")
        sandbox_agent = self._require_owned(
            processes.sandbox_agent,
            "actrail-sb",
        )
        gateway.terminate(grace_seconds=3)
        processes.gateway = None
        time.sleep(timing.sender_io_timeout_seconds + 0.25)
        disconnected_alert_id = self._latest_alert_id(alert_path)
        disconnected_evidence_count = alert_path.evidence_database.record_count()
        disconnected_window = max(
            timing.resource_poll_seconds * 2,
            timing.reconnect_interval_seconds + timing.resource_poll_seconds,
        )
        self._verify_disconnected_no_storage(
            alert_path,
            sandbox_agent,
            disconnected_window,
            after_alert_id=disconnected_alert_id,
            expected_evidence_count=disconnected_evidence_count,
        )

        reconnect_floor_ms = int(time.time() * 1000)
        reconnected_gateway = self._start_gateway()
        processes.gateway = reconnected_gateway
        reconnect = profile.connect(host_cid=1, port=self._config.vsock_port)
        self._wait_reconnected_resource_alert(
            alert_path,
            reconnected_gateway,
            sandbox_agent,
            disconnected_alert_id,
            reconnect_floor_ms,
        )
        return reconnect

    @staticmethod
    def _require_owned(
        process: ManagedProcess | None,
        name: str,
    ) -> ManagedProcess:
        if process is None:
            raise RuntimeError(f"{name} process ownership is missing")
        return process

    def _prepare_assets(self, provider_port: int) -> None:
        self._assets.mkdir(parents=True)
        self._coord.mkdir()
        sources = {
            "named-agent-root": Path(__file__).parent / "assets/named_agent_root.py",
            "workload.sh": Path(__file__).parent / "assets/workload.sh",
            "provider-proxy.py": (
                self._config.repo / "tests/support/llm-http-proxy/provider_proxy.py"
            ),
        }
        for name, source in sources.items():
            if not source.is_file():
                raise RuntimeError(f"host sandbox alert asset is missing: {source}")
            shutil.copy2(source, self._assets / name)
        for name in ("named-agent-root", "workload.sh"):
            (self._assets / name).chmod(0o755)
        (self._assets / "xiaoo.toml").write_text(
            self._xiaoo_config(provider_port),
            encoding="utf-8",
        )
        with (self._assets / "task.bin").open("wb") as output:
            output.truncate(2 * 1024 * 1024)

    def _write_gateway_config(self, daemon_port: int) -> None:
        self._run_config_init(
            self._config.bin_dir / "actrail-vsock-gateway",
            (
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
        )

    def _run_config_init(self, binary: Path, arguments: tuple[str, ...]) -> None:
        result = self._runner.run(
            (str(binary), "init", *arguments),
            timeout=self._config.command_timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "current release config generator failed: " + result.diagnostic
            )

    def _start_gateway(self) -> ManagedProcess:
        gateway = self._runner.start(
            (
                str(self._config.bin_dir / "actrail-vsock-gateway"),
                "--config",
                str(self._run_dir / "gateway.toml"),
            )
        )
        try:
            self._require_alive(gateway, "gateway")
        except Exception:
            gateway.terminate(grace_seconds=1)
            raise
        return gateway

    def _workload_environment(self) -> dict[str, str]:
        environment = dict(self._agent.environment)
        environment.update(
            {
                "ACTRAIL_HOST_NAMED_ROOT": str(self._assets / "named-agent-root"),
                "ACTRAIL_HOST_REAL_XIAOO": str(self._agent.binary),
                "ACTRAIL_HOST_XIAOO_CONFIG": str(self._assets / "xiaoo.toml"),
                "ACTRAIL_HOST_PROVIDER_SCRIPT": str(self._assets / "provider-proxy.py"),
                "ACTRAIL_HOST_COORD_DIR": str(self._coord),
                "ACTRAIL_HOST_TASK_FILE": str(self._assets / "task.bin"),
                "ACTRAIL_HOST_ROOT_PID_FILE": str(self._coord / "root.pid"),
                "ACTRAIL_HOST_CHILD_RELEASE": str(self._coord / "child.release"),
                "ACTRAIL_HOST_CHILD_TIMEOUT_SECONDS": str(
                    self._config.ready_timeout_seconds
                ),
                "ACTRAIL_HOST_READY_TIMEOUT_SECONDS": str(
                    self._config.ready_timeout_seconds
                ),
                "ACTRAIL_HOST_PROVIDER_PORT": str(self._provider_port),
            }
        )
        return environment

    def _wait_memory_baseline(
        self,
        alert_path: SandboxAlertPath,
        gateway: ManagedProcess,
        sb: ManagedProcess,
    ) -> None:
        expected = "sandbox.resource.oom_risk"
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self._require_alive(gateway, "gateway")
            self._require_alive(sb, "actrail-sb")
            observed = {record.category for record in alert_path.database.records()}
            if expected in observed:
                return
            time.sleep(0.1)
        raise RuntimeError("timed out waiting for host memory-risk alert")

    def _verify_disconnected_no_storage(
        self,
        alert_path: SandboxAlertPath,
        sb: ManagedProcess,
        observation_window_seconds: float,
        *,
        after_alert_id: int | None = None,
        expected_evidence_count: int | None = None,
    ) -> None:
        alert_id = (
            self._latest_alert_id(alert_path)
            if after_alert_id is None
            else after_alert_id
        )
        evidence_count = (
            alert_path.evidence_database.record_count()
            if expected_evidence_count is None
            else expected_evidence_count
        )
        time.sleep(observation_window_seconds)
        self._require_alive(sb, "disconnected actrail-sb")
        new_alerts = alert_path.database.records(after_alert_id=alert_id)
        if new_alerts:
            raise AssertionError(
                "disconnected actrail-sb persisted sandbox alerts: "
                + ", ".join(record.category for record in new_alerts)
            )
        current_evidence_count = alert_path.evidence_database.record_count()
        if current_evidence_count != evidence_count:
            raise AssertionError(
                "disconnected actrail-sb persisted sandbox evidence: "
                f"before={evidence_count}, after={current_evidence_count}"
            )

    def _wait_reconnected_resource_alert(
        self,
        alert_path: SandboxAlertPath,
        gateway: ManagedProcess,
        sb: ManagedProcess,
        after_alert_id: int,
        reconnect_floor_ms: int,
    ) -> SandboxAlertRecord:
        expected = "sandbox.resource.oom_risk"
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self._require_alive(gateway, "reconnected gateway")
            self._require_alive(sb, "reconnected actrail-sb")
            records = alert_path.database.records(after_alert_id=after_alert_id)
            replayed = [
                record
                for record in records
                if record.detected_at_ms < reconnect_floor_ms
            ]
            if replayed:
                raise AssertionError(
                    "actrail-sb replayed disconnected observations after reconnect: "
                    + ", ".join(record.category for record in replayed)
                )
            for record in records:
                if record.category == expected:
                    return record
            time.sleep(0.1)
        raise RuntimeError(
            "timed out waiting for a newly sampled resource alert after reconnect"
        )

    @staticmethod
    def _latest_alert_id(alert_path: SandboxAlertPath) -> int:
        records = alert_path.database.records()
        return records[-1].alert_id if records else 0

    def _wait_all_alerts(
        self,
        alert_path: SandboxAlertPath,
        gateway: ManagedProcess,
        sb: ManagedProcess,
        root_marker: tuple[int, int, str],
        child_release_ms: int,
    ) -> dict[str, SandboxAlertRecord]:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self._require_alive(gateway, "gateway")
            self._require_alive(sb, "actrail-sb")
            selected = self._select_records(
                alert_path,
                root_marker,
                child_release_ms,
            )
            if set(self._CATEGORIES).issubset(selected):
                return {category: selected[category] for category in self._CATEGORIES}
            time.sleep(0.1)
        raise RuntimeError(
            "timed out waiting for host memory-risk, high-read and "
            f"high-write alerts for named root pid={root_marker[0]}"
        )

    @staticmethod
    def _select_records(
        alert_path: SandboxAlertPath,
        root_marker: tuple[int, int, str],
        child_release_ms: int,
    ) -> dict[str, SandboxAlertRecord]:
        selected: dict[str, SandboxAlertRecord] = {}
        root_pid, root_start_time_ticks, root_name_hex = root_marker
        for record in alert_path.database.records():
            if record.gateway_id <= 0 or record.sb_id <= 0:
                raise AssertionError(f"invalid sandbox alert source: {record}")
            if record.category.startswith("sandbox.process."):
                if record.detected_at_ms < child_release_ms:
                    continue
                if record.process != {
                    "pid": root_pid,
                    "start_time_ticks": root_start_time_ticks,
                    "executable_name_hex": root_name_hex,
                }:
                    continue
                bytes_observed = record.extras.get("bytes")
                threshold = record.extras.get("threshold_bytes")
                if not isinstance(bytes_observed, int) or not isinstance(
                    threshold,
                    int,
                ):
                    raise AssertionError(f"I/O alert bytes are missing: {record}")
                if bytes_observed <= threshold:
                    raise AssertionError(
                        f"I/O alert did not cross its threshold: {record}"
                    )
                expected_threshold = (
                    SandboxResourceAlertHostScenario._READ_THRESHOLD_BYTES
                    if record.category == "sandbox.process.high_read"
                    else SandboxResourceAlertHostScenario._WRITE_THRESHOLD_BYTES
                )
                if threshold != expected_threshold:
                    raise AssertionError(f"I/O alert threshold changed: {record}")
            selected.setdefault(record.category, record)
        return selected

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

    def _wait_tcp(self, port: int) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError("hand-observation listener did not become ready")

    def _read_root_marker(self) -> tuple[int, int, str]:
        raw = (self._coord / "root.pid").read_text(encoding="ascii").strip()
        if not raw.isdigit() or int(raw) <= 0:
            raise RuntimeError(f"named Agent root PID is invalid: {raw!r}")
        pid = int(raw)
        stat = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
        fields = stat.rsplit(") ", 1)
        if len(fields) != 2:
            raise RuntimeError(f"named Agent root stat is invalid: {stat!r}")
        tail = fields[1].split()
        if len(tail) <= 19:
            raise RuntimeError(f"named Agent root stat is truncated: {stat!r}")
        start_time_ticks = int(tail[19])
        name = Path(f"/proc/{pid}/comm").read_bytes().rstrip(b"\n")
        if name != b"actrail-root":
            raise RuntimeError(f"named Agent root comm is invalid: {name!r}")
        name_hex = name.ljust(16, b"\0").hex()
        return pid, start_time_ticks, name_hex

    @staticmethod
    def _require_alive(process: ManagedProcess, name: str) -> None:
        time.sleep(0.1)
        if process.poll() is not None:
            result = process.wait(timeout=1)
            raise RuntimeError(f"{name} exited early: {result.diagnostic}")

    @staticmethod
    def _xiaoo_config(provider_port: int) -> str:
        return f"""[llm]
provider = "deepseek"
model = "deepseek-chat"
api_key_env = "ACTRAIL_VIRTUAL_XIAOO_API_KEY"
api_base = "http://127.0.0.1:{provider_port}"
max_tokens = 128
context_window = 32768
reasoning_effort = "off"
"""
