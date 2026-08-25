from __future__ import annotations

import sqlite3
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any


_CATEGORY_BY_KIND = {
    1: "sandbox.resource.high_cpu",
    2: "sandbox.resource.oom_killed",
    3: "sandbox.resource.oom_risk",
    4: "sandbox.process.high_read",
    5: "sandbox.process.high_write",
}


@dataclass(frozen=True)
class SandboxAlertRecord:
    """One structured alert read from actraild's sandbox alert store."""

    alert_id: int
    ingest_epoch: int
    gateway_id: int
    sb_id: int
    batch_sequence: int
    observation_index: int
    category: str
    detected_at_ms: int
    persisted_at_ms: int
    boot_id: str
    process: dict[str, int | str] | None
    extras: dict[str, Any]

    @property
    def delivery_source(self) -> dict[str, Any]:
        sandbox: dict[str, Any] = {
            "gateway_id": self.gateway_id,
            "sb_id": self.sb_id,
            "boot_id": self.boot_id,
        }
        if self.process is not None:
            sandbox["process"] = self.process
        return {"sandbox": sandbox}


class SandboxAlertDatabase:
    """Read-only view of the production sandbox-alert SQLite database."""

    def __init__(self, path: Path):
        self._path = path.resolve()

    def records(self, *, after_alert_id: int = 0) -> list[SandboxAlertRecord]:
        if not self._path.is_file():
            return []
        connection = sqlite3.connect(
            f"file:{self._path}?mode=ro",
            uri=True,
            timeout=1,
        )
        try:
            self._verify_schema(connection)
            rows = connection.execute(
                "SELECT alert_id, ingest_epoch, gateway_id, sb_id, "
                "batch_sequence, observation_index, alert_kind, "
                "detected_at_ms, persisted_at_ms, payload "
                "FROM sandbox_alerts WHERE alert_id > ? ORDER BY alert_id",
                (after_alert_id,),
            ).fetchall()
        finally:
            connection.close()
        return [self._decode_row(row) for row in rows]

    def assert_independent_from(self, main_database: Path) -> None:
        main_database = main_database.resolve()
        if main_database == self._path:
            raise AssertionError("sandbox alerts share the main database path")
        connection = sqlite3.connect(
            f"file:{main_database}?mode=ro",
            uri=True,
            timeout=1,
        )
        try:
            row = connection.execute(
                "SELECT 1 FROM sqlite_master "
                "WHERE type = 'table' AND name = 'sandbox_alerts'"
            ).fetchone()
        finally:
            connection.close()
        if row is not None:
            raise AssertionError("main database contains the sandbox_alerts table")

    @staticmethod
    def _verify_schema(connection: sqlite3.Connection) -> None:
        row = connection.execute(
            "SELECT schema_version FROM sandbox_alert_schema_meta "
            "WHERE singleton = 1"
        ).fetchone()
        if row != (2,):
            raise AssertionError(f"unsupported sandbox alert schema: {row}")

    @classmethod
    def _decode_row(cls, row: tuple[Any, ...]) -> SandboxAlertRecord:
        (
            alert_id,
            ingest_epoch,
            gateway_id,
            sb_id,
            batch_sequence,
            observation_index,
            kind,
            detected_at_ms,
            persisted_at_ms,
            payload,
        ) = row
        category = _CATEGORY_BY_KIND.get(kind)
        if category is None:
            raise AssertionError(f"unknown sandbox alert kind: {kind}")
        reader = _PayloadReader(bytes(payload))
        boot_id = str(uuid.UUID(bytes=reader.take(16)))
        process: dict[str, int | str] | None = None
        extras = {
            "batch_sequence": cls._u64(batch_sequence, "batch_sequence"),
            "observation_index": int(observation_index),
        }
        if kind == 1:
            extras["usage_basis_points"] = reader.u16()
            extras["threshold_basis_points"] = reader.u16()
        elif kind == 2:
            extras["victim_pid"] = reader.u32()
            extras["victim_comm"] = reader.take(16).split(b"\0", 1)[0].decode(
                "utf-8", errors="replace"
            )
            attribution = reader.u8()
            root_present = reader.u8()
            if reader.take(2) != b"\0\0":
                raise AssertionError("OOM alert reserved bytes are non-zero")
            root = {
                "pid": reader.u32(),
                "start_time_ticks": reader.u64(),
                "executable_name_hex": reader.take(16).hex(),
            }
            extras["attribution"] = {
                0: "unknown",
                1: "monitored",
                2: "unmonitored",
            }.get(attribution)
            if extras["attribution"] is None:
                raise AssertionError("invalid OOM alert attribution")
            if root_present == 1:
                process = root
            elif root_present != 0 or any(
                (root["pid"], root["start_time_ticks"])
            ) or root["executable_name_hex"] != "00" * 16:
                raise AssertionError("invalid OOM monitored root marker")
        elif kind == 3:
            extras["available_bytes"] = reader.u64()
            extras["threshold_bytes"] = reader.u64()
        else:
            process = {
                "pid": reader.u32(),
                "start_time_ticks": reader.u64(),
                "executable_name_hex": reader.take(16).hex(),
            }
            extras["sample_started_ms"] = reader.u64()
            extras["bytes"] = reader.u64()
            extras["threshold_bytes"] = reader.u64()
        reader.finish()
        return SandboxAlertRecord(
            alert_id=int(alert_id),
            ingest_epoch=cls._u64(ingest_epoch, "ingest_epoch"),
            gateway_id=int(gateway_id),
            sb_id=int(sb_id),
            batch_sequence=extras["batch_sequence"],
            observation_index=int(observation_index),
            category=category,
            detected_at_ms=cls._u64(detected_at_ms, "detected_at_ms"),
            persisted_at_ms=cls._u64(persisted_at_ms, "persisted_at_ms"),
            boot_id=boot_id,
            process=process,
            extras=extras,
        )

    @staticmethod
    def _u64(raw: Any, field: str) -> int:
        value = bytes(raw)
        if len(value) != 8:
            raise AssertionError(f"sandbox alert {field} has invalid width")
        return int.from_bytes(value, "big")


class _PayloadReader:
    def __init__(self, payload: bytes):
        self._payload = payload
        self._offset = 0

    def take(self, length: int) -> bytes:
        end = self._offset + length
        value = self._payload[self._offset:end]
        if len(value) != length:
            raise AssertionError("sandbox alert payload is truncated")
        self._offset = end
        return value

    def u16(self) -> int:
        return int.from_bytes(self.take(2), "big")

    def u8(self) -> int:
        return self.take(1)[0]

    def u32(self) -> int:
        return int.from_bytes(self.take(4), "big")

    def u64(self) -> int:
        return int.from_bytes(self.take(8), "big")

    def finish(self) -> None:
        if self._offset != len(self._payload):
            raise AssertionError("sandbox alert payload has trailing bytes")
