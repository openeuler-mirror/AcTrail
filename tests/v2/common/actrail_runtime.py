from __future__ import annotations

import json
import os
import subprocess
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import TestOutput


@dataclass(frozen=True)
class CommandResult:
    argv: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str

    @property
    def output(self) -> str:
        return self.stdout + self.stderr


@dataclass(frozen=True)
class AlertForwardingRuntimePaths:
    proxy_executable: Path
    proxy_config: Path
    plugin_config: Path
    socket_path: Path


class ActrailRuntime:
    def __init__(
        self,
        repo: Path,
        bin_dir: Path,
        command_timeout_seconds: int,
        output: TestOutput,
        operator_config: Path | None = None,
        operator_config_patch: Path | None = None,
        clean_control_state: bool = True,
    ):
        self._repo = repo
        self._bin_dir = bin_dir if bin_dir.is_absolute() else repo / bin_dir
        self._command_timeout_seconds = command_timeout_seconds
        self._output = output
        self._operator_config = operator_config
        self._operator_config_patch = operator_config_patch
        self.actraild = self._require_binary("actraild")
        self.actrailctl = (
            self._require_binary("actrailctl") if clean_control_state else None
        )
        self.actrailviewer = self._require_binary("actrailviewer")
        self._started = False

    @classmethod
    def isolated(
        cls,
        repo: Path,
        bin_dir: Path,
        command_timeout_seconds: int,
        output: TestOutput,
        work_dir: Path,
        *,
        hand_observation_listen_addr: str | None = None,
        sandbox_alerts_database: Path | None = None,
        alert_forwarding: AlertForwardingRuntimePaths | None = None,
        clean_control_state: bool = True,
    ) -> "ActrailRuntime":
        work_dir = work_dir.resolve()
        if not work_dir.is_dir():
            raise RuntimeError(
                f"isolated AcTrail work directory is missing: {work_dir}"
            )
        operator_config = work_dir / "actraild.conf"
        operator_config_patch = work_dir / "actraild.patch.toml"
        cls.write_isolated_operator_config_patch(
            operator_config_patch,
            work_dir,
            hand_observation_listen_addr=hand_observation_listen_addr,
            sandbox_alerts_database=sandbox_alerts_database,
            alert_forwarding=alert_forwarding,
        )
        return cls(
            repo,
            bin_dir,
            command_timeout_seconds,
            output,
            operator_config,
            operator_config_patch,
            clean_control_state,
        )

    def prepare(self) -> list[CommandResult]:
        results = [
            self.run_checked(self._init_command()),
            self.run_checked([*self._daemon_command(), "stop"]),
        ]
        if self.actrailctl is not None:
            results.append(
                self.run_checked([*self._control_command(), "clean"])
            )
        results.append(self.run_checked([*self._daemon_command(), "start"]))
        self._started = True
        return results

    def stop(self) -> CommandResult | None:
        if not self._started:
            return None
        result = self.run([*self._daemon_command(), "stop"])
        if result.returncode == 0:
            self._started = False
        return result

    def clean(self, *, echo: bool = True) -> CommandResult:
        return self.run([*self._control_command(), "clean"], echo=echo)

    def control_command(self, *arguments: Path | str) -> list[Path | str]:
        return [*self._control_command(), *arguments]

    def viewer_command(self, *arguments: Path | str) -> list[Path | str]:
        command: list[Path | str] = [self.actrailviewer]
        if self._operator_config is not None:
            command.extend(["--config", self._operator_config])
        return [*command, *arguments]

    def run(
        self,
        argv: list[Path | str],
        *,
        timeout_seconds: int | None = None,
        environment: dict[str, str] | None = None,
        echo: bool = True,
        cwd: Path | None = None,
    ) -> CommandResult:
        command = tuple(str(argument) for argument in argv)
        completed = subprocess.run(
            command,
            cwd=cwd or self._repo,
            env=environment,
            capture_output=True,
            text=True,
            timeout=timeout_seconds or self._command_timeout_seconds,
            check=False,
        )
        if echo:
            self._output.command_output(completed.stdout, completed.stderr)
        return CommandResult(
            argv=command,
            returncode=completed.returncode,
            stdout=completed.stdout,
            stderr=completed.stderr,
        )

    def run_checked(
        self,
        argv: list[Path | str],
        *,
        timeout_seconds: int | None = None,
        environment: dict[str, str] | None = None,
        echo: bool = True,
    ) -> CommandResult:
        result = self.run(
            argv,
            timeout_seconds=timeout_seconds,
            environment=environment,
            echo=echo,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"command exited with {result.returncode}: {' '.join(result.argv)}\n"
                f"stdout={result.stdout}\nstderr={result.stderr}"
            )
        return result

    def _require_binary(self, name: str) -> Path:
        binary = self._bin_dir / name
        if not binary.is_file():
            raise RuntimeError(f"release binary not found: {binary}")
        return binary

    def _init_command(self) -> list[Path | str]:
        command = [*self._daemon_command(), "init", "-f"]
        if self._operator_config_patch is not None:
            command.extend(["--patch", self._operator_config_patch])
        return command

    def _daemon_command(self) -> list[Path | str]:
        command: list[Path | str] = [self.actraild]
        if self._operator_config is not None:
            command.extend(["--config", self._operator_config])
        return command

    def _control_command(self) -> list[Path | str]:
        if self.actrailctl is None:
            raise RuntimeError("actrailctl is disabled for this isolated runtime")
        command: list[Path | str] = [self.actrailctl]
        if self._operator_config is not None:
            command.extend(["--config", self._operator_config])
        return command

    @staticmethod
    def write_isolated_operator_config_patch(
        path: Path,
        work_dir: Path,
        *,
        plugin_directory: Path | None = None,
        payload_tls_enabled: bool | None = None,
        payload_stdio_enabled: bool | None = None,
        payload_tls_seccomp_syscalls: list[str] | None = None,
        payload_socket_seccomp_syscalls: list[str] | None = None,
        hand_observation_listen_addr: str | None = None,
        sandbox_alerts_database: Path | None = None,
        alert_forwarding: AlertForwardingRuntimePaths | None = None,
    ) -> None:
        quoted = {
            name: json.dumps(str(work_dir / relative))
            for name, relative in {
                "socket": "run/control.sock",
                "pid": "run/actraild.pid",
                "log": "log/actraild.log",
                "storage": "data/actrail.sqlite",
                "sandbox_evidence": "data/sandbox-evidence.sqlite",
                "export": "data/export",
                "tls_sync": "run/tls-sync.sock",
                "cluster_spool": "data/cluster-spool",
                "cluster_state": "data/cluster-report-state.sqlite",
                "cluster_root": "data/cluster",
                "plugins": "plugins",
            }.items()
        }
        if plugin_directory is not None:
            quoted["plugins"] = json.dumps(str(plugin_directory.resolve()))
        default_shutdown_wait_ms = 150100
        if hand_observation_listen_addr is not None:
            default_shutdown_wait_ms += 10000
        if sandbox_alerts_database is not None:
            default_shutdown_wait_ms += 10000
        shutdown_wait_ms = int(
            os.environ.get(
                "ACTRAIL_V2_SHUTDOWN_WAIT_MS",
                str(default_shutdown_wait_ms),
            )
        )
        supervision_poll_interval_ms = int(
            os.environ.get("ACTRAIL_V2_SUPERVISION_POLL_INTERVAL_MS", "100")
        )
        finalization_shutdown_drain_timeout_ms = int(
            os.environ.get(
                "ACTRAIL_V2_FINALIZATION_SHUTDOWN_DRAIN_TIMEOUT_MS",
                "30000",
            )
        )
        post_trace_broker_reply_timeout_ms = int(
            os.environ.get(
                "ACTRAIL_V2_POST_TRACE_BROKER_REPLY_TIMEOUT_MS",
                "5000",
            )
        )
        post_trace_shutdown_drain_timeout_ms = int(
            os.environ.get(
                "ACTRAIL_V2_POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS",
                "30000",
            )
        )
        plugin_alert_drain_timeout_ms = int(
            os.environ.get(
                "ACTRAIL_V2_PLUGIN_ALERT_DRAIN_TIMEOUT_MS",
                "30000",
            )
        )
        timing_values = {
            "ACTRAIL_V2_SHUTDOWN_WAIT_MS": shutdown_wait_ms,
            "ACTRAIL_V2_SUPERVISION_POLL_INTERVAL_MS": supervision_poll_interval_ms,
            "ACTRAIL_V2_FINALIZATION_SHUTDOWN_DRAIN_TIMEOUT_MS": (
                finalization_shutdown_drain_timeout_ms
            ),
            "ACTRAIL_V2_POST_TRACE_BROKER_REPLY_TIMEOUT_MS": (
                post_trace_broker_reply_timeout_ms
            ),
            "ACTRAIL_V2_POST_TRACE_SHUTDOWN_DRAIN_TIMEOUT_MS": (
                post_trace_shutdown_drain_timeout_ms
            ),
            "ACTRAIL_V2_PLUGIN_ALERT_DRAIN_TIMEOUT_MS": (
                plugin_alert_drain_timeout_ms
            ),
        }
        for name, value in timing_values.items():
            if value < 1:
                raise ValueError(f"{name} must be positive")
        payload_tls = (
            f"sync_event_socket_path = {quoted['tls_sync']}\n"
        )
        if payload_tls_enabled is not None:
            payload_tls += (
                f"enabled = {str(payload_tls_enabled).lower()}\n"
            )
        if payload_tls_seccomp_syscalls is not None:
            payload_tls += (
                "seccomp_syscalls = ["
                + ", ".join(
                    json.dumps(syscall)
                    for syscall in payload_tls_seccomp_syscalls
                )
                + "]\n"
            )
        payload_socket = ""
        if payload_socket_seccomp_syscalls is not None:
            payload_socket = (
                "\n[payload.socket]\n"
                "seccomp_syscalls = ["
                + ", ".join(
                    json.dumps(syscall)
                    for syscall in payload_socket_seccomp_syscalls
                )
                + "]\n"
            )
        payload_stdio = ""
        if payload_stdio_enabled is not None:
            payload_stdio = (
                "\n[payload.stdio]\n"
                f"enabled = {str(payload_stdio_enabled).lower()}\n"
            )
        hand_observation = ""
        if hand_observation_listen_addr is not None:
            hand_observation = (
                "\n[hand_observation]\n"
                "enabled = true\n"
                f"listen_addr = {json.dumps(hand_observation_listen_addr)}\n"
                "connection_poll_interval_ms = 100\n"
                "\n[sandbox_evidence]\n"
                f"path = {quoted['sandbox_evidence']}\n"
            )
        sandbox_alerts = ""
        if sandbox_alerts_database is not None:
            sandbox_alerts = (
                "\n[sandbox_alerts]\n"
                "enabled = true\n"
                f"path = {json.dumps(str(sandbox_alerts_database.resolve()))}\n"
            )
        alert_forwarding_document = ""
        if alert_forwarding is not None:
            alert_forwarding_document = (
                "\n[alert_forwarding]\n"
                f"proxy_executable = {json.dumps(str(alert_forwarding.proxy_executable))}\n"
                f"proxy_config_path = {json.dumps(str(alert_forwarding.proxy_config))}\n"
                f"plugin_config_path = {json.dumps(str(alert_forwarding.plugin_config))}\n"
                f"socket_path = {json.dumps(str(alert_forwarding.socket_path))}\n"
                "queue_capacity = 64\n"
                "read_timeout_ms = 250\n"
                "write_timeout_ms = 250\n"
                "heartbeat_interval_ms = 1000\n"
                "heartbeat_ack_timeout_ms = 500\n"
                "startup_timeout_ms = 5000\n"
                "startup_poll_interval_ms = 20\n"
                "max_frame_bytes = 262144\n"
                "max_trace_id_bytes = 128\n"
                "max_category_bytes = 128\n"
                "max_description_bytes = 4096\n"
                "max_extras_bytes = 131072\n"
                "link_thread_stack_bytes = 524288\n"
            )
        path.write_text(
            "[control]\n"
            f"socket_path = {quoted['socket']}\n"
            f"pid_file = {quoted['pid']}\n"
            f"log_path = {quoted['log']}\n"
            "\n[supervision]\n"
            f"shutdown_wait_ms = {shutdown_wait_ms}\n"
            f"poll_interval_ms = {supervision_poll_interval_ms}\n"
            "\n[control.finalization]\n"
            "shutdown_drain_timeout_ms = "
            f"{finalization_shutdown_drain_timeout_ms}\n"
            "\n[control.finalization.post_trace]\n"
            f"broker_reply_timeout_ms = {post_trace_broker_reply_timeout_ms}\n"
            "shutdown_drain_timeout_ms = "
            f"{post_trace_shutdown_drain_timeout_ms}\n"
            "\n[storage.sqlite]\n"
            f"path = {quoted['storage']}\n"
            "\n[storage.retention]\n"
            "enabled = false\n"
            "\n[export.snapshot]\n"
            f"directory = {quoted['export']}\n"
            "\n[payload.tls]\n"
            + payload_tls
            + payload_socket
            + payload_stdio
            + "\n[cluster.report]\n"
            f"spool_dir = {quoted['cluster_spool']}\n"
            f"state_path = {quoted['cluster_state']}\n"
            "\n[cluster.center]\n"
            f"root_dir = {quoted['cluster_root']}\n"
            "\n[plugins.discovery]\n"
            f"directory = {quoted['plugins']}\n"
            "\n[plugins.alerts]\n"
            f"drain_timeout_ms = {plugin_alert_drain_timeout_ms}\n"
            + hand_observation
            + sandbox_alerts
            + alert_forwarding_document,
            encoding="utf-8",
        )
