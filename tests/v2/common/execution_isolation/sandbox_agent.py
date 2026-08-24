from __future__ import annotations

import stat
import time
import tomllib
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.process import CommandResult, ManagedProcess, SubprocessRunner


@dataclass(frozen=True)
class SandboxAgentTiming:
    resource_poll_seconds: float
    sender_io_timeout_seconds: float
    reconnect_interval_seconds: float

    @property
    def disconnected_observation_window_seconds(self) -> float:
        return max(
            self.resource_poll_seconds * 2,
            self.sender_io_timeout_seconds
            + self.reconnect_interval_seconds
            + self.resource_poll_seconds,
        )


class SandboxAgentProfile:
    """Owns test-local actrail-sb config, argv and control readiness."""

    def __init__(
        self,
        *,
        binary: Path,
        work_dir: Path,
        runner: SubprocessRunner,
        command_timeout_seconds: float,
    ) -> None:
        self._binary = binary.resolve()
        self._runner = runner
        self._command_timeout_seconds = command_timeout_seconds
        self.config_path = (work_dir / "sb.toml").resolve()
        self.control_socket = (work_dir / "actrail-sb-control.sock").resolve()
        self.instance_lock = (work_dir / "actrail-sb.lock").resolve()

    def refresh_default_config(self, root_process_name: str) -> SandboxAgentTiming:
        result = self._runner.run(
            (
                str(self._binary),
                "init",
                "--output",
                str(self.config_path),
                "--root-process-name",
                root_process_name,
                "--control-socket",
                str(self.control_socket),
                "--instance-lock-path",
                str(self.instance_lock),
            ),
            timeout=self._command_timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError(
                "current release actrail-sb config generator failed: "
                + result.diagnostic
            )
        document = tomllib.loads(self.config_path.read_text(encoding="utf-8"))
        self._validate_test_paths(document)
        self._reject_embedded_endpoint(document)
        return SandboxAgentTiming(
            resource_poll_seconds=self._positive_milliseconds(
                document, "sampler", "poll_interval_ms"
            ),
            sender_io_timeout_seconds=self._positive_milliseconds(
                document, "sender", "io_timeout_ms"
            ),
            reconnect_interval_seconds=self._positive_milliseconds(
                document, "sender", "reconnect_interval_ms"
            ),
        )

    def daemon_argv(self) -> tuple[str, ...]:
        return (
            str(self._binary),
            "daemon",
            "--config",
            str(self.config_path),
        )

    def connect(
        self,
        *,
        host_cid: int,
        port: int,
    ) -> CommandResult:
        result = self._runner.run(
            (
                str(self._binary),
                "connect",
                "--control-socket",
                str(self.control_socket),
                "--host-cid",
                str(host_cid),
                "--port",
                str(port),
            ),
            timeout=self._command_timeout_seconds,
        )
        if result.returncode != 0:
            raise RuntimeError("actrail-sb connect failed: " + result.diagnostic)
        output = result.stdout.strip()
        required = (
            "actrail-sb connected sb_id=",
            " generation=",
            " reused=",
        )
        if not all(marker in output for marker in required):
            raise RuntimeError(
                "actrail-sb connect omitted connection evidence: " + output
            )
        return result

    def wait_ready(self, process: ManagedProcess, timeout_seconds: float) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if process.poll() is not None:
                result = process.wait(timeout=1)
                raise RuntimeError(
                    "actrail-sb exited before control readiness: " + result.diagnostic
                )
            try:
                mode = self.control_socket.stat().st_mode
            except FileNotFoundError:
                time.sleep(0.05)
                continue
            if stat.S_ISSOCK(mode):
                return
            raise RuntimeError(
                f"actrail-sb control path is not a socket: {self.control_socket}"
            )
        raise RuntimeError("timed out waiting for actrail-sb control socket")

    def _validate_test_paths(self, document: dict[str, object]) -> None:
        control = document.get("control")
        if not isinstance(control, dict):
            raise RuntimeError("generated actrail-sb config omitted [control]")
        if control.get("socket_path") != str(self.control_socket):
            raise RuntimeError(
                "generated actrail-sb config ignored test control socket"
            )
        if document.get("instance_lock_path") != str(self.instance_lock):
            raise RuntimeError("generated actrail-sb config ignored test instance lock")

    @classmethod
    def _reject_embedded_endpoint(cls, value: object) -> None:
        if isinstance(value, dict):
            for key, child in value.items():
                if key in {"host_cid", "port"}:
                    raise RuntimeError(
                        "actrail-sb daemon config must not contain a VSOCK endpoint"
                    )
                cls._reject_embedded_endpoint(child)
        elif isinstance(value, list):
            for child in value:
                cls._reject_embedded_endpoint(child)

    @staticmethod
    def _positive_milliseconds(
        document: dict[str, object],
        section_name: str,
        field_name: str,
    ) -> float:
        section = document.get(section_name)
        if not isinstance(section, dict):
            raise RuntimeError(f"generated actrail-sb config omitted [{section_name}]")
        milliseconds = section.get(field_name)
        if not isinstance(milliseconds, int) or milliseconds <= 0:
            raise RuntimeError(
                f"generated actrail-sb config has invalid {section_name}.{field_name}"
            )
        return milliseconds / 1000
