from __future__ import annotations

import json
import os
import shlex
import signal
import time
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.actrail_runtime import AlertForwardingRuntimePaths

from .constants import DEFAULTS


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
            subscriber_host=DEFAULTS.subscriber_host,
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
        self.write_forwarding_config(enabled=DEFAULTS.forwarding_enabled, categories=list(DEFAULTS.forwarding_categories))

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
            "all_categories": DEFAULTS.forwarding_all_categories,
            "categories": categories,
        }
        self.plugin_config.write_text(
            json.dumps(document, indent=2, sort_keys=True) + "\n",
            encoding="utf-8",
        )

    def terminate(self, timeout_seconds: float = DEFAULTS.stop_timeout_seconds) -> None:
        pid = self._read_pid()
        if pid is None or not self._is_owned_process(pid):
            return
        os.kill(pid, signal.SIGTERM)
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if not self._process_exists(pid):
                return
            time.sleep(DEFAULTS.poll_seconds)
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
            f'socket_mode_octal = "{DEFAULTS.socket_mode_octal}"\n'
            f"allowed_uids = [{uid}]\n"
            f"allowed_gids = [{gid}]\n"
            f"connection_limit = {DEFAULTS.ingress_connection_limit}\n"
            f"accept_poll_interval_ms = {DEFAULTS.accept_poll_interval_ms}\n"
            f"io_poll_interval_ms = {DEFAULTS.io_poll_interval_ms}\n"
            f"producer_idle_timeout_ms = {DEFAULTS.producer_idle_timeout_ms}\n"
            f"max_frame_bytes = {DEFAULTS.max_frame_bytes}\n"
            f"max_trace_id_bytes = {DEFAULTS.max_trace_id_bytes}\n"
            f"max_category_bytes = {DEFAULTS.max_category_bytes}\n"
            f"max_description_bytes = {DEFAULTS.max_description_bytes}\n"
            f"max_extras_bytes = {DEFAULTS.max_extras_bytes}\n"
            f"worker_thread_stack_bytes = {DEFAULTS.worker_thread_stack_bytes}\n"
            "\n[subscriber]\n"
            f'listen_addr = "{self.subscriber_host}:{self.subscriber_port}"\n'
            f"allow_insecure_remote = {str(DEFAULTS.allow_insecure_remote).lower()}\n"
            f"listen_backlog = {DEFAULTS.listen_backlog}\n"
            f"connection_limit = {DEFAULTS.subscriber_connection_limit}\n"
            f"accept_poll_interval_ms = {DEFAULTS.accept_poll_interval_ms}\n"
            f"io_poll_interval_ms = {DEFAULTS.io_poll_interval_ms}\n"
            f"max_frame_bytes = {DEFAULTS.max_frame_bytes}\n"
            f"max_client_id_bytes = {DEFAULTS.max_client_id_bytes}\n"
            f"max_request_id_bytes = {DEFAULTS.max_request_id_bytes}\n"
            f"max_topics = {DEFAULTS.max_topics}\n"
            f"max_topic_bytes = {DEFAULTS.max_topic_bytes}\n"
            f"broadcast_queue_capacity = {DEFAULTS.broadcast_queue_capacity}\n"
            f"broadcaster_thread_stack_bytes = {DEFAULTS.broadcaster_thread_stack_bytes}\n"
            f"queue_capacity = {DEFAULTS.queue_capacity}\n"
            f"heartbeat_interval_ms = {DEFAULTS.heartbeat_interval_ms}\n"
            f"pong_timeout_ms = {DEFAULTS.pong_timeout_ms}\n"
            f"peer_idle_timeout_ms = {DEFAULTS.peer_idle_timeout_ms}\n"
            f"worker_thread_stack_bytes = {DEFAULTS.worker_thread_stack_bytes}\n"
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
