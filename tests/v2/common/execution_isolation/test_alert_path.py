from __future__ import annotations

import json
import tempfile
import unittest
from dataclasses import replace
from pathlib import Path
from types import SimpleNamespace
from typing import Callable
from unittest.mock import patch

from tests.v2.common.execution_isolation.alert_path import (
    RESOURCE_ALERT_INSTANCE_ID,
    SandboxAlertPath,
    SandboxAlertThresholds,
)
from tests.v2.common.sandbox_alert_database import SandboxAlertRecord


class _SubscriberBoundary:
    def __init__(self, messages: list[dict[str, object]]) -> None:
        self._messages = messages

    def wait_for_alert(
        self,
        timeout_seconds: float,
        predicate: Callable[[dict[str, object]], bool],
    ) -> dict[str, object]:
        del timeout_seconds
        for message in self._messages:
            if predicate(message):
                return message
        raise AssertionError("matching alert not received")

    def assert_no_alert(
        self,
        timeout_seconds: float,
        predicate: Callable[[dict[str, object]], bool] | None = None,
    ) -> None:
        del timeout_seconds
        for message in self._messages:
            if "cat" in message and (
                predicate is None or predicate(message)
            ):
                raise AssertionError(f"unexpected forwarded alert: {message}")


class _LifecycleSubscriber:
    def connect(self, timeout_seconds: float) -> None:
        del timeout_seconds

    def subscribe(
        self,
        request_id: str,
        topics: list[str],
        severities: list[str],
        timeout_seconds: float,
    ) -> None:
        del request_id, topics, severities, timeout_seconds

    def wait_for_heartbeat(self, timeout_seconds: float) -> None:
        del timeout_seconds

    def close(self) -> None:
        pass


class _LifecycleProxy:
    subscriber_address = ("127.0.0.1", 48182)
    token = "test-token"
    runtime_paths = SimpleNamespace()

    def write_forwarding_config(
        self,
        *,
        enabled: bool,
        categories: list[str],
    ) -> None:
        del enabled, categories

    def require_running(self) -> int:
        return 912

    def terminate(self) -> None:
        pass


class _LifecycleRuntime:
    actraild = Path("/test/actraild")

    def prepare(self) -> None:
        pass

    def run_checked(self, argv: list[object]) -> SimpleNamespace:
        del argv
        return SimpleNamespace(output=f"loaded instance={RESOURCE_ALERT_INSTANCE_ID}")

    def stop(self) -> None:
        pass


class _DatabaseBoundary:
    def __init__(self, records: list[SandboxAlertRecord]) -> None:
        self._records = records

    def records(self) -> list[SandboxAlertRecord]:
        return list(self._records)


class SandboxAlertPathTest(unittest.TestCase):
    def test_waits_for_oom_delivery_matching_victim_pid(self) -> None:
        root_marker: dict[str, int | str] = {
            "pid": 412,
            "start_time_ticks": 88_001,
            "executable_name_hex": "6163747261696c2d726f6f7400000000",
        }
        expected = {
            "cat": "sandbox.resource.oom_killed",
            "s": "critical",
            "ts": 1_725_000_000_123,
            "source": {
                "sandbox": {
                    "gateway_id": 7,
                    "sb_id": 9,
                    "boot_id": "d172df2c-76c9-47b0-abd4-84d044af6141",
                    "process": root_marker,
                }
            },
            "extras": {
                "victim_pid": 731,
                "victim_comm": "python3",
                "attribution": "monitored",
            },
        }
        path = self._path_with_messages(
            [
                {
                    **expected,
                    "cat": "sandbox.resource.oom_risk",
                    "extras": {**expected["extras"], "victim_pid": 729},
                },
                {
                    **expected,
                    "extras": {**expected["extras"], "victim_pid": 730},
                },
                {
                    **expected,
                    "source": {
                        "sandbox": {
                            **expected["source"]["sandbox"],
                            "process": {**root_marker, "pid": 411},
                        }
                    },
                    "extras": {**expected["extras"], "victim_pid": 728},
                },
                expected,
            ]
        )

        delivery = path.wait_for_oom_killed_delivery(
            5,
            victim_pid=731,
        )

        self.assertIs(delivery, expected)

    def test_rejects_a_second_public_delivery(self) -> None:
        duplicate = {
            "cat": "sandbox.resource.oom_killed",
            "extras": {"victim_pid": 731},
        }
        path = self._path_with_messages([duplicate])

        with self.assertRaisesRegex(AssertionError, "unexpected forwarded alert"):
            path.assert_no_delivery(0.25)

    def test_duplicate_check_ignores_an_unrelated_host_oom(self) -> None:
        root_marker: dict[str, int | str] = {
            "pid": 412,
            "start_time_ticks": 88_001,
            "executable_name_hex": "6163747261696c2d726f6f7400000000",
        }
        unrelated = {
            "cat": "sandbox.resource.oom_killed",
            "source": {
                "sandbox": {
                    "process": {**root_marker, "pid": 999},
                }
            },
            "extras": {"victim_pid": 998},
        }
        path = self._path_with_messages([unrelated])

        path.assert_no_matching_oom_killed_delivery(
            0.25,
            victim_pid=731,
        )

    def test_duplicate_check_rejects_same_victim_with_wrong_root(self) -> None:
        root_marker: dict[str, int | str] = {
            "pid": 412,
            "start_time_ticks": 88_001,
            "executable_name_hex": "6163747261696c2d726f6f7400000000",
        }
        duplicate = {
            "cat": "sandbox.resource.oom_killed",
            "source": {
                "sandbox": {
                    "process": {**root_marker, "pid": 999},
                }
            },
            "extras": {"victim_pid": 731},
        }
        path = self._path_with_messages([duplicate])

        with self.assertRaisesRegex(AssertionError, "unexpected forwarded alert"):
            path.assert_no_matching_oom_killed_delivery(
                0.25,
                victim_pid=731,
            )

    def test_public_delivery_resolves_to_its_persisted_record(self) -> None:
        root_marker = {
            "pid": 412,
            "start_time_ticks": 88_001,
            "executable_name_hex": "6163747261696c2d726f6f7400000000",
        }
        expected = SandboxAlertRecord(
            alert_id=19,
            ingest_epoch=4,
            gateway_id=7,
            sb_id=9,
            batch_sequence=12,
            observation_index=3,
            category="sandbox.resource.oom_killed",
            detected_at_ms=1_725_000_000_123,
            persisted_at_ms=1_725_000_000_130,
            boot_id="d172df2c-76c9-47b0-abd4-84d044af6141",
            process=dict(root_marker),
            extras={
                "batch_sequence": 12,
                "observation_index": 3,
                "victim_pid": 731,
                "victim_comm": "python3",
                "attribution": "monitored",
            },
        )
        unrelated = replace(
            expected,
            alert_id=18,
            extras={**expected.extras, "victim_pid": 730},
        )
        delivery = {
            "cat": "sandbox.resource.oom_killed",
            "s": "critical",
            "ts": 1_725_000_000_123,
            "source": {
                "sandbox": {
                    "gateway_id": 7,
                    "sb_id": 9,
                    "boot_id": "d172df2c-76c9-47b0-abd4-84d044af6141",
                    "process": dict(root_marker),
                }
            },
            "extras": {
                "batch_sequence": 12,
                "observation_index": 3,
                "victim_pid": 731,
                "victim_comm": "python3",
                "attribution": "monitored",
            },
        }
        path = SandboxAlertPath.__new__(SandboxAlertPath)
        path._database = _DatabaseBoundary(  # type: ignore[assignment]
            [unrelated, expected]
        )

        record = path.assert_persisted_delivery(delivery)

        self.assertIs(record, expected)

    def test_public_delivery_rejects_duplicate_persisted_records(self) -> None:
        record = SandboxAlertRecord(
            alert_id=19,
            ingest_epoch=4,
            gateway_id=7,
            sb_id=9,
            batch_sequence=12,
            observation_index=3,
            category="sandbox.resource.oom_killed",
            detected_at_ms=1_725_000_000_123,
            persisted_at_ms=1_725_000_000_130,
            boot_id="d172df2c-76c9-47b0-abd4-84d044af6141",
            process=None,
            extras={
                "batch_sequence": 12,
                "observation_index": 3,
                "victim_pid": 731,
                "victim_comm": "python3",
                "attribution": "unmonitored",
            },
        )
        delivery = {
            "cat": "sandbox.resource.oom_killed",
            "s": "critical",
            "ts": 1_725_000_000_123,
            "source": {
                "sandbox": {
                    "gateway_id": 7,
                    "sb_id": 9,
                    "boot_id": "d172df2c-76c9-47b0-abd4-84d044af6141",
                }
            },
            "extras": {
                "batch_sequence": 12,
                "observation_index": 3,
                "victim_pid": 731,
                "victim_comm": "python3",
                "attribution": "unmonitored",
            },
        }
        path = SandboxAlertPath.__new__(SandboxAlertPath)
        path._database = _DatabaseBoundary(  # type: ignore[assignment]
            [record, replace(record, alert_id=20)]
        )

        with self.assertRaisesRegex(AssertionError, "found 2"):
            path.assert_persisted_delivery(delivery)

    def test_web_disabled_path_starts_and_stops_without_web_control(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            default = (
                root
                / "examples/plugins/builtin/sandbox-resource-alert/"
                "sandbox-resource-alert.config.json"
            )
            default.parent.mkdir(parents=True)
            default.write_text(json.dumps({}), encoding="utf-8")
            runtime = _LifecycleRuntime()
            proxy = _LifecycleProxy()
            subscriber = _LifecycleSubscriber()
            with (
                patch(
                    "tests.v2.common.execution_isolation.alert_path."
                    "AlertProxyTestProfile.create",
                    return_value=proxy,
                ),
                patch(
                    "tests.v2.common.execution_isolation.alert_path."
                    "ActrailRuntime.isolated",
                    return_value=runtime,
                ),
                patch(
                    "tests.v2.common.execution_isolation.alert_path."
                    "AlertSubscriberClient",
                    return_value=subscriber,
                ),
                patch(
                    "tests.v2.common.execution_isolation.alert_path."
                    "SandboxResourceAlertWebControl",
                    side_effect=AssertionError("Web control was constructed"),
                ),
            ):
                path = SandboxAlertPath(
                    repo=root,
                    bin_dir=root / "bin",
                    work_dir=root,
                    context=SimpleNamespace(output=object()),  # type: ignore[arg-type]
                    command_timeout_seconds=10,
                    daemon_port=43182,
                    subscriber_port=48182,
                    web_port=None,
                    categories=("sandbox.resource.oom_killed",),
                    thresholds=SandboxAlertThresholds(
                        cpu_usage_basis_points=10_000,
                        memory_available_bytes=1,
                        read_interval_bytes=1,
                        write_interval_bytes=1,
                    ),
                )

                path.start(1)
                errors = path.stop()

        self.assertEqual(errors, [])

    def test_web_updates_report_when_web_control_is_disabled(self) -> None:
        path = SandboxAlertPath.__new__(SandboxAlertPath)
        path._web = None

        operations = (
            path.update_memory_threshold,
            path.assert_failed_update_preserves_memory_threshold,
        )
        for operation in operations:
            with self.subTest(operation=operation.__name__):
                with self.assertRaisesRegex(
                    RuntimeError,
                    "^actrailweb is disabled for this alert path$",
                ):
                    operation(1)

    @staticmethod
    def _path_with_messages(messages: list[dict[str, object]]) -> SandboxAlertPath:
        path = SandboxAlertPath.__new__(SandboxAlertPath)
        path._subscriber = _SubscriberBoundary(messages)  # type: ignore[assignment]
        return path


if __name__ == "__main__":
    unittest.main()
