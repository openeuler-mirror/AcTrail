from __future__ import annotations

import json
import os
import shlex
import signal
import time
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.actrail_runtime import AlertForwardingRuntimePaths


@dataclass(frozen=True)
class AlertProxyTestProfile:
    work_dir: Path
    executable: Path
    wrapper: Path
    pid_file: Path
    config: Path
    plugin_config: Path
    socket_path: Path
    subscriber_host: str
    subscriber_port: int
    token: str

    @classmethod
    def create(
        cls,
        work_dir: Path,
        executable: Path,
        subscriber_port: int,
        token: str,
    ) -> "AlertProxyTestProfile":
        resolved_executable = executable.resolve()
        if not resolved_executable.is_file():
            raise RuntimeError(
                f"release binary not found: {resolved_executable}"
            )
        profile = cls(
            work_dir=work_dir.resolve(),
            executable=resolved_executable,
            wrapper=(work_dir / "run-alert-proxy.sh").resolve(),
            pid_file=(work_dir / "run" / "alert-proxy.pid").resolve(),
            config=(work_dir / "actraild-alert-proxy.toml").resolve(),
            plugin_config=(work_dir / "alert-forwarding.json").resolve(),
            socket_path=(work_dir / "run" / "alert-proxy.sock").resolve(),
            subscriber_host="127.0.0.1",
            subscriber_port=subscriber_port,
            token=token,
        )
        profile.write()
        return profile

    @property
    def runtime_paths(self) -> AlertForwardingRuntimePaths:
        return AlertForwardingRuntimePaths(
            proxy_executable=self.wrapper,
            proxy_config=self.config,
            plugin_config=self.plugin_config,
            socket_path=self.socket_path,
        )

    @property
    def subscriber_address(self) -> tuple[str, int]:
        return self.subscriber_host, self.subscriber_port

    def write(self) -> None:
        self.pid_file.parent.mkdir(parents=True, exist_ok=True)
        self._write_wrapper()
        self._write_proxy_config()
        self.write_forwarding_config(enabled=False, categories=["consecutive_failure"])

    def require_running(self) -> int:
        pid = self._read_pid()
        if pid is None or not self._is_owned_process(pid):
            raise AssertionError("daemon did not auto-launch the configured alert proxy")
        return pid

    def write_forwarding_config(
        self,
        *,
        enabled: bool,
        categories: list[str],
    ) -> None:
        document = {
            "enabled": enabled,
            "all_categories": False,
            "categories": categories,
        }
        self.plugin_config.write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    def terminate(self, timeout_seconds: float = 5.0) -> None:
        pid = self._read_pid()
        if pid is None or not self._is_owned_process(pid):
            return
        os.kill(pid, signal.SIGTERM)
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if not self._process_exists(pid):
                return
            time.sleep(0.05)
        if self._is_owned_process(pid):
            os.kill(pid, signal.SIGKILL)

    def _write_wrapper(self) -> None:
        self.wrapper.write_text(
            "#!/bin/sh\n"
            f"printf '%s\\n' \"$$\" > {shlex.quote(str(self.pid_file))}\n"
            f"exec {shlex.quote(str(self.executable))} \"$@\"\n",
            encoding="utf-8",
        )
        self.wrapper.chmod(0o700)

    def _write_proxy_config(self) -> None:
        uid = os.getuid()
        gid = os.getgid()
        self.config.write_text(
            "[daemon_ingress]\n"
            f"socket_path = {json.dumps(str(self.socket_path))}\n"
            'socket_mode_octal = "0600"\n'
            f"allowed_uids = [{uid}]\n"
            f"allowed_gids = [{gid}]\n"
            "connection_limit = 2\n"
            "accept_poll_interval_ms = 20\n"
            "io_poll_interval_ms = 100\n"
            "producer_idle_timeout_ms = 5000\n"
            "max_frame_bytes = 262144\n"
            "max_trace_id_bytes = 128\n"
            "max_category_bytes = 128\n"
            "max_description_bytes = 4096\n"
            "max_extras_bytes = 131072\n"
            "worker_thread_stack_bytes = 524288\n"
            "\n[subscriber]\n"
            f'listen_addr = "{self.subscriber_host}:{self.subscriber_port}"\n'
            "allow_insecure_remote = false\n"
            "listen_backlog = 16\n"
            "connection_limit = 8\n"
            "accept_poll_interval_ms = 20\n"
            "io_poll_interval_ms = 100\n"
            "max_frame_bytes = 262144\n"
            "max_client_id_bytes = 128\n"
            "max_request_id_bytes = 128\n"
            "max_topics = 16\n"
            "max_topic_bytes = 128\n"
            "broadcast_queue_capacity = 64\n"
            "broadcaster_thread_stack_bytes = 524288\n"
            "queue_capacity = 16\n"
            "heartbeat_interval_ms = 1000\n"
            "pong_timeout_ms = 500\n"
            "peer_idle_timeout_ms = 5000\n"
            "worker_thread_stack_bytes = 524288\n"
            f"allowed_tokens = [{json.dumps(self.token)}]\n",
            encoding="utf-8",
        )

    def _read_pid(self) -> int | None:
        try:
            return int(self.pid_file.read_text(encoding="utf-8").strip())
        except (FileNotFoundError, ValueError):
            return None

    def _is_owned_process(self, pid: int) -> bool:
        try:
            command = Path(f"/proc/{pid}/cmdline").read_bytes()
        except OSError:
            return False
        return (
            str(self.executable).encode() in command
            and str(self.config).encode() in command
        )

    @staticmethod
    def _process_exists(pid: int) -> bool:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return False
        return True
