from __future__ import annotations

import subprocess
import tomllib
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import TestOutput
from tests.v2.common.patch_builder import PatchBuilder

from .constants import RUNTIME_DEFAULTS as DEFAULTS


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
        clean_control_state: bool = DEFAULTS.clean_control_state,
        operator_config_patches: list[Path] | None = None,
    ):
        self._repo = repo
        self._bin_dir = bin_dir if bin_dir.is_absolute() else repo / bin_dir
        self._command_timeout_seconds = command_timeout_seconds
        self._output = output
        self._operator_config = operator_config
        self._operator_config_patches = list(operator_config_patches or [])
        if operator_config_patch is not None:
            self._operator_config_patches.append(operator_config_patch)
        self.actraild = self._require_binary("actraild")
        self.actrailctl = self._require_binary("actrailctl")
        self._clean_control_state = clean_control_state
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
        operator_config_patch: Path | None = None,
        operator_config_patches: list[Path] | None = None,
        hand_observation_listen_addr: str | None = None,
        sandbox_alerts_database: Path | None = None,
        alert_forwarding: AlertForwardingRuntimePaths | None = None,
        agent_invocation_commands: list[Path] | None = None,
        clean_control_state: bool = DEFAULTS.clean_control_state,
    ) -> "ActrailRuntime":
        work_dir = work_dir.resolve()
        if not work_dir.is_dir():
            raise RuntimeError(
                f"isolated AcTrail work directory is missing: {work_dir}"
            )
        operator_config = work_dir / "actraild.conf"
        runtime_patch = work_dir / "actraild.patch.toml"
        cls.write_isolated_operator_config_patch(
            runtime_patch,
            work_dir,
            hand_observation_listen_addr=hand_observation_listen_addr,
            sandbox_alerts_database=sandbox_alerts_database,
            alert_forwarding=alert_forwarding,
            agent_invocation_commands=agent_invocation_commands,
        )
        patches = [runtime_patch]
        if operator_config_patch is not None:
            patches.append(operator_config_patch)
        patches.extend(operator_config_patches or [])
        return cls(
            repo,
            bin_dir,
            command_timeout_seconds,
            output,
            operator_config=operator_config,
            operator_config_patches=patches,
            clean_control_state=clean_control_state,
        )

    def prepare(self) -> list[CommandResult]:
        results = [
            self.run_checked(self._init_command()),
            self.run_checked([*self._daemon_command(), "stop"]),
        ]
        if self._clean_control_state:
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

    def clean(self, *, echo: bool = DEFAULTS.echo_commands) -> CommandResult:
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
        echo: bool = DEFAULTS.echo_commands,
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
        echo: bool = DEFAULTS.echo_commands,
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
        command = [*self._control_command(), "init", "-f"]
        for patch in self._operator_config_patches:
            command.extend(["--patch", patch])
        return command

    @property
    def database_path(self) -> Path:
        if self._operator_config is None:
            raise RuntimeError("operator config path is required")
        with self._operator_config.open("rb") as source:
            return Path(tomllib.load(source)["storage"]["sqlite"]["path"])

    def _daemon_command(self) -> list[Path | str]:
        command: list[Path | str] = [self.actraild]
        if self._operator_config is not None:
            command.extend(["--config", self._operator_config])
        return command

    def _control_command(self) -> list[Path | str]:
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
        agent_invocation_commands: list[Path] | None = None,
    ) -> None:
        builder = PatchBuilder().with_work_dir(work_dir)
        if plugin_directory is not None:
            builder.with_plugin_directory(plugin_directory)
        shutdown_wait_ms = 150100
        if hand_observation_listen_addr is not None:
            shutdown_wait_ms += 10000
        if sandbox_alerts_database is not None:
            shutdown_wait_ms += 10000
        builder.with_runtime_timing(shutdown_wait_ms).with_values({
            "storage": {"retention": {"enabled": False}},
        })
        if payload_tls_enabled is not None:
            builder.with_values({"payload": {"tls": {"enabled": payload_tls_enabled}}})
        if payload_stdio_enabled is not None:
            builder.with_values({"payload": {"stdio": {"enabled": payload_stdio_enabled}}})
        if payload_tls_seccomp_syscalls is not None:
            builder.with_values({"payload": {"tls": {"seccomp_syscalls": payload_tls_seccomp_syscalls}}})
        if payload_socket_seccomp_syscalls is not None:
            builder.with_values({"payload": {"socket": {"seccomp_syscalls": payload_socket_seccomp_syscalls}}})
        if hand_observation_listen_addr is not None:
            builder.with_values({
                "hand_observation": {"enabled": True, "listen_addr": hand_observation_listen_addr, "connection_poll_interval_ms": 100},
                "sandbox_evidence": {"path": str(work_dir / "data/sandbox-evidence.sqlite")},
            })
        if sandbox_alerts_database is not None:
            builder.with_values({"sandbox_alerts": {"enabled": True, "path": str(sandbox_alerts_database.resolve())}})
        if alert_forwarding is not None:
            builder.with_values({"alert_forwarding": {
                "proxy_executable": str(alert_forwarding.proxy_executable),
                "proxy_config_path": str(alert_forwarding.proxy_config),
                "plugin_config_path": str(alert_forwarding.plugin_config),
                "socket_path": str(alert_forwarding.socket_path),
                "queue_capacity": 64,
                "read_timeout_ms": 250,
                "write_timeout_ms": 250,
                "heartbeat_interval_ms": 1000,
                "heartbeat_ack_timeout_ms": 500,
                "startup_timeout_ms": 5000,
                "startup_poll_interval_ms": 20,
                "max_frame_bytes": 262144,
                "max_trace_id_bytes": 128,
                "max_category_bytes": 128,
                "max_description_bytes": 4096,
                "max_extras_bytes": 131072,
                "link_thread_stack_bytes": 524288,
            }})
        if agent_invocation_commands is not None:
            builder.with_values({"agent_invocation": {"commands": [str(command.resolve()) for command in agent_invocation_commands]}})
        builder.build().save(path)
