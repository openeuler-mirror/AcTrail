from __future__ import annotations

import json
import queue
import socket
import struct
import threading
import time
from pathlib import Path
from typing import Any, Callable


class AlertSubscriberClient:
    """A bounded client for the production alert subscriber protocol."""

    def __init__(
        self,
        address: tuple[str, int],
        token: str,
        client_id: str,
        transcript: Path,
        *,
        max_payload_bytes: int = 262_140,
    ):
        self._address = address
        self._token = token
        self._client_id = client_id
        self._transcript = transcript
        self._max_payload_bytes = max_payload_bytes
        self._socket: socket.socket | None = None
        self._messages: queue.Queue[dict[str, Any] | BaseException] = queue.Queue(
            maxsize=64
        )
        self._send_lock = threading.Lock()
        self._record_lock = threading.Lock()
        self._heartbeat_seen = threading.Event()
        self._stopping = threading.Event()
        self._reader: threading.Thread | None = None
        self._reader_failure: BaseException | None = None

    def connect(self, timeout_seconds: float) -> None:
        deadline = time.monotonic() + timeout_seconds
        last_error: OSError | None = None
        while time.monotonic() < deadline:
            try:
                connection = socket.create_connection(
                    self._address,
                    timeout=min(0.5, max(0.05, deadline - time.monotonic())),
                )
            except OSError as error:
                last_error = error
                time.sleep(0.05)
                continue
            connection.settimeout(0.5)
            self._socket = connection
            try:
                self._handshake(deadline)
                self._start_reader()
            except Exception:
                self.close()
                raise
            return
        raise RuntimeError(
            f"alert subscriber could not connect to {self._address}: {last_error}"
        )

    def subscribe(
        self,
        request_id: str,
        topics: list[str],
        severities: list[str],
        timeout_seconds: float,
    ) -> None:
        self._send(
            {
                "id": request_id,
                "action": "subscribe",
                "topics": topics,
                "filter": {"severity": severities, "tags": {}},
            }
        )
        deadline = time.monotonic() + timeout_seconds
        response = self._receive_application_message(deadline)
        if response.get("status") != "accepted" or response.get("id") != request_id:
            raise AssertionError(f"subscription rejected: {response}")
        if response.get("subscribed_topics") != topics:
            raise AssertionError(f"subscription topics changed: {response}")

    def wait_for_heartbeat(self, timeout_seconds: float) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if self._heartbeat_seen.wait(min(0.05, deadline - time.monotonic())):
                return
            if self._reader_failure is not None:
                raise ConnectionError(
                    f"alert subscriber reader failed: {self._reader_failure}"
                ) from self._reader_failure
        raise AssertionError("subscriber did not complete a heartbeat exchange")

    def wait_for_alert(
        self,
        timeout_seconds: float,
        predicate: Callable[[dict[str, Any]], bool],
    ) -> dict[str, Any]:
        deadline = time.monotonic() + timeout_seconds
        observed: list[dict[str, Any]] = []
        while time.monotonic() < deadline:
            message = self._receive_application_message(deadline)
            if "cat" not in message:
                raise AssertionError(f"unexpected subscriber message: {message}")
            observed.append(message)
            if predicate(message):
                return message
        raise AssertionError(f"matching alert not received; observed={observed}")

    def wait_for_matching_alerts(
        self,
        timeout_seconds: float,
        predicates: dict[str, Callable[[dict[str, Any]], bool]],
    ) -> dict[str, dict[str, Any]]:
        """Collect unordered alerts until each named production predicate matches."""
        deadline = time.monotonic() + timeout_seconds
        pending = dict(predicates)
        matched: dict[str, dict[str, Any]] = {}
        observed: list[dict[str, Any]] = []
        while pending and time.monotonic() < deadline:
            message = self._receive_application_message(deadline)
            if "cat" not in message:
                raise AssertionError(f"unexpected subscriber message: {message}")
            observed.append(message)
            for name, predicate in tuple(pending.items()):
                if predicate(message):
                    matched[name] = message
                    del pending[name]
                    break
        if pending:
            raise AssertionError(
                "matching alerts not received; "
                f"pending={sorted(pending)} observed={observed}"
            )
        return matched

    def assert_no_alert(
        self,
        timeout_seconds: float,
        predicate: Callable[[dict[str, Any]], bool] | None = None,
    ) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            try:
                message = self._receive_application_message(deadline)
            except TimeoutError:
                return
            if "cat" in message and (
                predicate is None or predicate(message)
            ):
                raise AssertionError(f"unexpected forwarded alert: {message}")

    def close(self) -> None:
        connection = self._socket
        self._socket = None
        if connection is not None:
            self._stopping.set()
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass
            connection.close()
        reader = self._reader
        self._reader = None
        if reader is not None:
            reader.join(timeout=2)

    def _handshake(self, deadline: float) -> None:
        self._send(
            {
                "action": "handshake",
                "version": "v1",
                "auth": {"token": self._token},
                "client_id": self._client_id,
            },
            record=False,
        )
        response = self._receive(deadline)
        if response.get("status") != "success":
            raise AssertionError(f"subscriber handshake failed: {response}")
        if not response.get("session_id"):
            raise AssertionError(f"subscriber handshake omitted session id: {response}")
        heartbeat = response.get("heartbeat_interval")
        if not isinstance(heartbeat, int) or heartbeat <= 0:
            raise AssertionError(f"subscriber handshake has invalid heartbeat: {response}")

    def _receive_application_message(self, deadline: float) -> dict[str, Any]:
        while time.monotonic() < deadline:
            try:
                item = self._messages.get(timeout=max(0.01, deadline - time.monotonic()))
            except queue.Empty as error:
                raise TimeoutError("subscriber receive deadline expired") from error
            if isinstance(item, BaseException):
                raise ConnectionError(f"alert subscriber reader failed: {item}") from item
            message = item
            if message.get("status") == "error":
                raise AssertionError(f"subscriber protocol error: {message}")
            return message
        raise TimeoutError("subscriber receive deadline expired")

    def _send(self, message: dict[str, Any], *, record: bool = True) -> None:
        connection = self._require_socket()
        payload = json.dumps(
            message,
            separators=(",", ":"),
            ensure_ascii=False,
        ).encode("utf-8")
        if len(payload) > self._max_payload_bytes:
            raise ValueError("subscriber request exceeds configured frame limit")
        with self._send_lock:
            connection.sendall(struct.pack(">I", len(payload)) + payload)
        if record:
            self._record("out", message)

    def _receive(self, deadline: float) -> dict[str, Any]:
        header = self._receive_exact(4, deadline)
        payload_length = struct.unpack(">I", header)[0]
        if payload_length > self._max_payload_bytes:
            raise AssertionError(
                f"subscriber frame length {payload_length} exceeds limit"
            )
        payload = self._receive_exact(payload_length, deadline)
        try:
            message = json.loads(payload)
        except json.JSONDecodeError as error:
            raise AssertionError("subscriber returned invalid JSON") from error
        if not isinstance(message, dict):
            raise AssertionError("subscriber returned non-object JSON")
        self._record("in", message)
        return message

    def _receive_exact(self, length: int, deadline: float) -> bytes:
        connection = self._require_socket()
        chunks = bytearray()
        while len(chunks) < length:
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("subscriber receive deadline expired")
            connection.settimeout(min(0.5, remaining))
            try:
                chunk = connection.recv(length - len(chunks))
            except socket.timeout:
                continue
            if not chunk:
                raise ConnectionError("alert subscriber connection closed")
            chunks.extend(chunk)
        return bytes(chunks)

    def _record(self, direction: str, message: dict[str, Any]) -> None:
        self._transcript.parent.mkdir(parents=True, exist_ok=True)
        with self._record_lock:
            with self._transcript.open("a", encoding="utf-8") as output:
                output.write(
                    json.dumps(
                        {"direction": direction, "message": message},
                        ensure_ascii=False,
                        sort_keys=True,
                    )
                    + "\n"
                )

    def _start_reader(self) -> None:
        self._reader = threading.Thread(
            target=self._read_loop,
            name=f"alert-subscriber-{self._client_id}",
            daemon=True,
        )
        self._reader.start()

    def _read_loop(self) -> None:
        try:
            while not self._stopping.is_set():
                message = self._receive(time.monotonic() + 3600)
                if message.get("action") == "ping":
                    nonce = message.get("nonce")
                    if not isinstance(nonce, int):
                        raise AssertionError(f"invalid heartbeat ping: {message}")
                    self._send(
                        {
                            "action": "pong",
                            "nonce": nonce,
                            "ts": int(time.time() * 1000),
                        }
                    )
                    self._heartbeat_seen.set()
                    continue
                self._messages.put_nowait(message)
        except BaseException as error:
            if not self._stopping.is_set():
                self._reader_failure = error
                try:
                    self._messages.put_nowait(error)
                except queue.Full:
                    pass

    def _require_socket(self) -> socket.socket:
        if self._socket is None:
            raise RuntimeError("alert subscriber is not connected")
        return self._socket
