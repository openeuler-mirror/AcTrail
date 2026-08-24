from __future__ import annotations

import json
import secrets
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.alert_proxy import AlertProxyTestProfile, AlertSubscriberClient
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.sandbox_alert_database import (
    SandboxAlertDatabase,
    SandboxAlertRecord,
)

from .evidence_database import SandboxEvidenceDatabase


@dataclass(frozen=True)
class SandboxAlertThresholds:
    cpu_usage_basis_points: int
    memory_available_bytes: int
    read_interval_bytes: int
    write_interval_bytes: int


class SandboxAlertPath:
    """Owns the real daemon, alert store, proxy and subscriber test path."""

    def __init__(
        self,
        *,
        repo: Path,
        bin_dir: Path,
        work_dir: Path,
        context: TestingContextSingleton,
        command_timeout_seconds: int,
        daemon_port: int,
        subscriber_port: int,
        categories: tuple[str, ...],
        thresholds: SandboxAlertThresholds,
    ) -> None:
        self._repo = repo
        self._bin_dir = bin_dir
        self._work_dir = work_dir
        self._context = context
        self._command_timeout_seconds = command_timeout_seconds
        self._daemon_port = daemon_port
        self._categories = categories
        self._thresholds = thresholds
        self._database = SandboxAlertDatabase(
            work_dir / "data" / "sandbox-alerts.sqlite"
        )
        self._evidence_database = SandboxEvidenceDatabase(
            work_dir / "data" / "sandbox-evidence.sqlite"
        )
        self._proxy = AlertProxyTestProfile.create(
            work_dir,
            bin_dir / "actraild-alert-proxy",
            subscriber_port,
            secrets.token_urlsafe(24),
        )
        self._proxy.write_forwarding_config(
            enabled=True,
            categories=list(categories),
        )
        self._runtime = ActrailRuntime.isolated(
            repo,
            bin_dir,
            command_timeout_seconds,
            context.output,
            work_dir,
            hand_observation_listen_addr=f"127.0.0.1:{daemon_port}",
            sandbox_alerts_database=work_dir / "data" / "sandbox-alerts.sqlite",
            alert_forwarding=self._proxy.runtime_paths,
            clean_control_state=False,
        )
        self._subscriber = AlertSubscriberClient(
            self._proxy.subscriber_address,
            self._proxy.token,
            f"sandbox-resource-alert-host-{secrets.token_hex(8)}",
            work_dir / "alert-subscriber.jsonl",
        )
        self._started = False

    @property
    def daemon_port(self) -> int:
        return self._daemon_port

    @property
    def database(self) -> SandboxAlertDatabase:
        return self._database

    @property
    def evidence_database(self) -> SandboxEvidenceDatabase:
        return self._evidence_database

    def start(self, ready_timeout_seconds: float) -> None:
        self._write_plugin_config()
        self._runtime.prepare()
        self._started = True
        self._proxy.require_running()
        self._subscriber.connect(ready_timeout_seconds)
        self._subscriber.subscribe(
            "sandbox-resource-alert-host",
            list(self._categories),
            [],
            ready_timeout_seconds,
        )
        self._subscriber.wait_for_heartbeat(ready_timeout_seconds)
        self._load_plugin()

    def stop(self) -> list[str]:
        errors: list[str] = []
        self._subscriber.close()
        if self._started:
            stopped = self._runtime.stop()
            if stopped is not None and stopped.returncode != 0:
                errors.append("daemon: " + stopped.output[-1000:])
            self._started = False
        try:
            self._proxy.terminate()
        except Exception as error:
            errors.append(f"alert-proxy: {error}")
        return errors

    def matching_deliveries(
        self,
        records: dict[str, SandboxAlertRecord],
        timeout_seconds: float,
    ) -> dict[str, dict[str, object]]:
        return self._subscriber.wait_for_matching_alerts(
            timeout_seconds,
            {
                category: (
                    lambda message, record=record: self._matches(record, message)
                )
                for category, record in records.items()
            },
        )

    def assert_delivery(
        self,
        record: SandboxAlertRecord,
        message: dict[str, object],
    ) -> None:
        source = message.get("source")
        if not isinstance(source, dict) or "trid" in source:
            raise AssertionError(f"sandbox delivery leaked trace identity: {message}")
        if not self._matches(record, message):
            raise AssertionError(
                "sandbox database record and subscriber delivery differ: "
                f"record={record}; delivery={message}"
            )
        if message.get("s") != "warning":
            raise AssertionError(f"sandbox host alert severity changed: {message}")

    def assert_independent_database(self) -> None:
        self._database.assert_independent_from(
            self._work_dir / "data" / "actrail.sqlite"
        )

    def _load_plugin(self) -> None:
        manifest = (
            self._repo
            / "examples/plugins/builtin/sandbox-resource-alert/"
            "sandbox-resource-alert.plugin.toml"
        )
        result = self._runtime.run_checked(
            [
                self._runtime.actraild,
                "--config",
                self._work_dir / "actraild.conf",
                "plugin",
                "load",
                "--manifest",
                manifest,
                "--plugin-config",
                self._work_dir / "sandbox-resource-alert.json",
                "--instance",
                "sandbox-resource-alert.host",
            ]
        )
        if "loaded instance=sandbox-resource-alert.host" not in result.output:
            raise RuntimeError("sandbox resource alert plugin did not become active")

    def _write_plugin_config(self) -> None:
        default = (
            self._repo
            / "examples/plugins/builtin/sandbox-resource-alert/"
            "sandbox-resource-alert.config.json"
        )
        document = json.loads(default.read_text(encoding="utf-8"))
        document.update(
            {
                "cpu_usage_threshold_basis_points": (
                    self._thresholds.cpu_usage_basis_points
                ),
                "memory_available_threshold_bytes": (
                    self._thresholds.memory_available_bytes
                ),
                "read_interval_threshold_bytes": (
                    self._thresholds.read_interval_bytes
                ),
                "write_interval_threshold_bytes": (
                    self._thresholds.write_interval_bytes
                ),
                "source_state_capacity": 64,
            }
        )
        (self._work_dir / "sandbox-resource-alert.json").write_text(
            json.dumps(document, indent=2) + "\n",
            encoding="utf-8",
        )

    @staticmethod
    def _matches(
        record: SandboxAlertRecord,
        message: dict[str, object],
    ) -> bool:
        return (
            message.get("cat") == record.category
            and message.get("ts") == record.detected_at_ms
            and message.get("source") == record.delivery_source
            and message.get("extras") == record.extras
        )
