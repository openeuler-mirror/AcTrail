from __future__ import annotations

import socket
import time
from pathlib import Path

from tests.v2.common.alert_proxy import AlertSubscriberClient
from tests.v2.common.kata_runtime import KataTestContainer
from tests.v2.common.process import ManagedProcess
from tests.v2.common.sandbox_alert_database import (
    SandboxAlertDatabase,
    SandboxAlertRecord,
)

from ..config import CloudHypervisorExecutionIsolationConfig
from ..identity import CloudHypervisorScenarioIdentity


class CloudHypervisorAlertVerifier:
    _EXPECTED_CATEGORIES = {
        "sandbox.resource.high_cpu",
        "sandbox.resource.oom_killed",
        "sandbox.resource.oom_risk",
        "sandbox.process.high_read",
        "sandbox.process.high_write",
    }

    def __init__(
        self,
        config: CloudHypervisorExecutionIsolationConfig,
        alerts: SandboxAlertDatabase,
        coordination: Path,
    ) -> None:
        self._config = config
        self._alerts = alerts
        self._coordination = coordination

    def wait_tcp(self, port: int) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError(
            CloudHypervisorScenarioIdentity.failure(
                "hand-observation TCP listener did not become ready"
            )
        )

    def wait_path(
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
                    CloudHypervisorScenarioIdentity.failure(
                        f"process exited before {description}: {result.diagnostic}"
                    )
                )
            time.sleep(0.05)
        raise RuntimeError(
            CloudHypervisorScenarioIdentity.failure(
                f"timed out waiting for {description}"
            )
        )

    def wait_observation_alerts(
        self,
        gateway: ManagedProcess,
        sb: ManagedProcess,
        root_pid: int,
        subscriber: AlertSubscriberClient,
    ) -> dict[str, SandboxAlertRecord]:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self.require_alive(gateway, "gateway")
            self.require_alive(sb, "actrail-sb")
            records = self._select_expected_alerts(root_pid)
            if self._EXPECTED_CATEGORIES.issubset(records):
                expected = {
                    category: records[category]
                    for category in self._EXPECTED_CATEGORIES
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
                        for category, record in expected.items()
                    },
                )
                for category, message in external.items():
                    self._assert_delivery(expected[category], message)
                return expected
            time.sleep(0.05)
        raise RuntimeError(
            CloudHypervisorScenarioIdentity.failure(
                "timed out waiting for Guest high-CPU, OOM-killed, OOM-risk, "
                "high-read and high-write alerts aggregated to "
                f"root pid={root_pid}"
            )
        )

    def trigger_guest_oom(self, vm: KataTestContainer) -> None:
        result = vm.exec(
            ("/bin/sh", "/opt/actrail-execution/oom-trigger.sh"),
            uid=0,
            gid=0,
            timeout=45,
        )
        if result.returncode != 0:
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    "controlled Guest OOM trigger failed: " + result.diagnostic
                )
            )
        if CloudHypervisorScenarioIdentity.OOM_KILL_MARKER not in result.stdout:
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    "controlled Guest OOM trigger omitted success evidence"
                )
            )

    def wait_resource_baseline(
        self,
        gateway: ManagedProcess,
        sb: ManagedProcess,
    ) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self.require_alive(gateway, "gateway")
            self.require_alive(sb, "actrail-sb")
            if any(
                record.category == "sandbox.resource.oom_risk"
                for record in self._alerts.records()
            ):
                return
            time.sleep(0.05)
        raise RuntimeError(
            CloudHypervisorScenarioIdentity.failure(
                "timed out waiting for the pre-OOM Guest resource baseline"
            )
        )

    def read_root_pid(self) -> int:
        raw = (self._coordination / "root.pid").read_text(
            encoding="ascii"
        ).strip()
        if not raw.isdigit() or int(raw) <= 0:
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    f"named Agent root PID is invalid: {raw!r}"
                )
            )
        return int(raw)

    def _select_expected_alerts(
        self,
        root_pid: int,
    ) -> dict[str, SandboxAlertRecord]:
        selected: dict[str, SandboxAlertRecord] = {}
        for record in self._alerts.records():
            if record.gateway_id <= 0 or record.sb_id <= 0:
                raise AssertionError(
                    CloudHypervisorScenarioIdentity.failure(
                        f"invalid sandbox alert source: {record}"
                    )
                )
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
            raise AssertionError(
                CloudHypervisorScenarioIdentity.failure(
                    f"sandbox delivery leaked trace identity: {message}"
                )
            )
        if not cls._matches_delivery(record, message):
            raise AssertionError(
                CloudHypervisorScenarioIdentity.failure(
                    "sandbox database record and subscriber delivery differ: "
                    f"record={record}; delivery={message}"
                )
            )
        expected_severity = (
            "critical"
            if record.category == "sandbox.resource.oom_killed"
            else "warning"
        )
        if message.get("s") != expected_severity:
            raise AssertionError(
                CloudHypervisorScenarioIdentity.failure(
                    f"sandbox alert severity changed: {message}"
                )
            )

    @staticmethod
    def require_alive(process: ManagedProcess, name: str) -> None:
        time.sleep(0.1)
        if process.poll() is not None:
            result = process.wait(timeout=1)
            raise RuntimeError(
                CloudHypervisorScenarioIdentity.failure(
                    f"{name} exited early: {result.diagnostic}"
                )
            )
