from __future__ import annotations

import base64
import shlex
import socket
import time
from pathlib import Path

from tests.v2.common.alert_proxy import AlertSubscriberClient
from tests.v2.common.kata_runtime import GuestConsole, KataTestContainer
from tests.v2.common.process import ManagedProcess
from tests.v2.common.sandbox_alert_database import (
    SandboxAlertDatabase,
    SandboxAlertRecord,
)

from ..config import CloudHypervisorExecutionIsolationConfig
from .transport import CoordinationDirectory, CoordinationFile, HostCoordination


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
        guest: GuestConsole,
    ) -> None:
        self._config = config
        self._alerts = alerts
        self._coordination = HostCoordination(coordination)
        self._guest = guest

    def wait_tcp(self, port: int) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            try:
                with socket.create_connection(("127.0.0.1", port), timeout=0.2):
                    return
            except OSError:
                time.sleep(0.05)
        raise RuntimeError(
            self._config.IDENTITY.failure(
                "hand-observation TCP listener did not become ready"
            )
        )

    def wait_path(
        self,
        path: CoordinationFile,
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
                    self._config.IDENTITY.failure(
                        f"process exited before {description}: {result.diagnostic}"
                    )
                )
            time.sleep(0.05)
        raise RuntimeError(
            self._config.IDENTITY.failure(
                f"timed out waiting for {description}"
            )
        )

    def wait_observation_alerts(
        self,
        gateway: ManagedProcess,
        root_pid: int,
        subscriber: AlertSubscriberClient,
    ) -> dict[str, SandboxAlertRecord]:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self.require_alive(gateway, "gateway")
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
            self._config.IDENTITY.failure(
                "timed out waiting for Guest high-CPU, OOM-killed, OOM-risk, "
                "high-read and high-write alerts aggregated to "
                f"root pid={root_pid}; "
                + self._observation_timeout_diagnostic(root_pid)
            )
        )

    def trigger_guest_oom(self, vm: KataTestContainer) -> None:
        script = (
            Path(__file__).parent.parent / "assets" / "oom-trigger.sh"
        ).read_bytes()
        payload = base64.b64encode(script).decode("ascii")
        marker = shlex.quote(self._config.IDENTITY.OOM_KILL_MARKER)
        result = self._guest.capture(
            vm.container_id,
            f"printf %s {shlex.quote(payload)} | /usr/bin/base64 -d | "
            "/usr/bin/env "
            f"ACTRAIL_EXECUTION_ISOLATION_OOM_KILL_MARKER={marker} /bin/sh",
            timeout=45,
        )
        if result.returncode != 0:
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    "controlled Guest OOM trigger failed: " + result.diagnostic
                )
            )
        if self._config.IDENTITY.OOM_KILL_MARKER not in result.stdout:
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    "controlled Guest OOM trigger omitted success evidence"
                )
            )

    def wait_resource_baseline(
        self,
        gateway: ManagedProcess,
    ) -> None:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        while time.monotonic() < deadline:
            self.require_alive(gateway, "gateway")
            if any(
                record.category == "sandbox.resource.oom_risk"
                for record in self._alerts.records()
            ):
                return
            time.sleep(0.05)
        raise RuntimeError(
            self._config.IDENTITY.failure(
                "timed out waiting for the pre-OOM Guest resource baseline"
            )
        )

    def read_root_pid(
        self,
        vm: KataTestContainer,
        coordination: CoordinationDirectory | None = None,
    ) -> int:
        source = coordination or self._coordination
        raw = source.file("root.pid").read_text(
            encoding="ascii"
        ).strip()
        if not raw.isdigit() or int(raw) <= 0:
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    f"named Agent root PID is invalid: {raw!r}"
                )
            )
        namespaced_pid = int(raw)
        command = (
            f"target={namespaced_pid}; "
            "for status in /proc/[0-9]*/status; do "
            "[ -r \"$status\" ] || continue; "
            "name=$(awk '$1 == \"Name:\" { print $2; exit }' \"$status\" "
            "2>/dev/null) || continue; "
            "[ \"$name\" = actrail-root ] || continue; "
            "inner=$(awk '$1 == \"NSpid:\" { print $NF; exit }' \"$status\" "
            "2>/dev/null) || continue; "
            "[ \"$inner\" = \"$target\" ] || continue; "
            "pid=${status#/proc/}; pid=${pid%/status}; "
            "printf '%s\\n' \"$pid\"; "
            "done"
        )
        resolved = self._guest.capture(
            vm.container_id,
            command,
            timeout=min(10.0, float(self._config.ready_timeout_seconds)),
        )
        candidates = [
            line.strip()
            for line in resolved.stdout.splitlines()
            if line.strip()
        ]
        if (
            resolved.returncode != 0
            or len(candidates) != 1
            or not candidates[0].isdigit()
            or int(candidates[0]) <= 0
        ):
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    "cannot resolve the named Agent root from workload PID "
                    f"namespace pid={namespaced_pid} into the Guest system PID "
                    "namespace: "
                    + (resolved.diagnostic or repr(candidates))
                )
            )
        return int(candidates[0])

    def _select_expected_alerts(
        self,
        root_pid: int,
    ) -> dict[str, SandboxAlertRecord]:
        selected: dict[str, SandboxAlertRecord] = {}
        for record in self._alerts.records():
            if record.gateway_id <= 0 or record.sb_id <= 0:
                raise AssertionError(
                    self._config.IDENTITY.failure(
                        f"invalid sandbox alert source: {record}"
                    )
                )
            if record.category.startswith("sandbox.process."):
                if record.process is None or record.process["pid"] != root_pid:
                    continue
            selected.setdefault(record.category, record)
        return selected

    def _observation_timeout_diagnostic(self, root_pid: int) -> str:
        records = self._alerts.records()
        selected = self._select_expected_alerts(root_pid)
        observed = sorted({record.category for record in records})
        missing = sorted(self._EXPECTED_CATEGORIES.difference(selected))
        candidates = sorted(
            (
                record.category,
                int(record.process["pid"]),
                int(record.process["start_time_ticks"]),
                str(record.process["executable_name_hex"]),
            )
            for record in records
            if record.category.startswith("sandbox.process.")
            and record.process is not None
        )
        candidate_text = [
            f"{category}(pid={pid},start={start},name={name})"
            for category, pid, start, name in candidates
        ]
        return (
            f"missing={missing}; observed={observed}; "
            f"process_candidates={candidate_text}"
        )

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

    def _assert_delivery(
        self,
        record: SandboxAlertRecord,
        message: dict[str, object],
    ) -> None:
        source = message.get("source")
        if not isinstance(source, dict) or "trid" in source:
            raise AssertionError(
                self._config.IDENTITY.failure(
                    f"sandbox delivery leaked trace identity: {message}"
                )
            )
        if not self._matches_delivery(record, message):
            raise AssertionError(
                self._config.IDENTITY.failure(
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
                self._config.IDENTITY.failure(
                    f"sandbox alert severity changed: {message}"
                )
            )

    def require_alive(self, process: ManagedProcess, name: str) -> None:
        time.sleep(0.1)
        if process.poll() is not None:
            result = process.wait(timeout=1)
            raise RuntimeError(
                self._config.IDENTITY.failure(
                    f"{name} exited early: {result.diagnostic}"
                )
            )
