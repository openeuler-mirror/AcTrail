"""Own daemon configuration, lifecycle and AcTrail-controlled process launches."""

from __future__ import annotations

import subprocess
import re
import os
import signal
import tomllib
from concurrent.futures import ThreadPoolExecutor
from contextvars import ContextVar
from contextlib import contextmanager
from dataclasses import dataclass
from enum import Enum
from pathlib import Path
from types import TracebackType
from typing import ClassVar, Literal, Self

from tests.e2e.common.core import TestOutput
from tests.e2e.common.patch_builder import PatchBuilder

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
class LaunchPermissions:
    """actrailctl 启动阶段报告的权限选择结果。"""

    requested: str
    selected: str
    degraded: bool


@dataclass(frozen=True)
class LaunchResult:
    """启动输出解析完成后的进程句柄及其 trace 身份。"""

    process: subprocess.Popen[str]
    trace_id: int
    permissions: LaunchPermissions | None = None
    required_capabilities: tuple[str, ...] = ()


@dataclass(frozen=True)
class AlertForwardingRuntimePaths:
    proxy_executable: Path
    proxy_config: Path
    plugin_config: Path
    socket_path: Path


class ToolAccess(Enum):
    """Requested Agent tool permissions, mapped by each CLI adapter."""

    READ_ONLY = "read_only"
    WRITABLE = "writable"
    NONE = "none"


@dataclass(frozen=True)
class LaunchOptions:
    """Observation settings and scoped Agent execution options."""

    name: str | None = None
    host_ebpf: Literal["auto", "required", "disabled"] = "required"
    seccomp_notify: Literal["auto", "required", "disabled"] = "auto"
    tool_access: ToolAccess | None = None
    max_turns: int | None = None

    def __post_init__(self) -> None:
        if self.max_turns is not None and self.max_turns < 1:
            raise ValueError("max_turns must be positive")

    def arguments(self) -> list[str]:
        arguments = [
            "--host-ebpf",
            self.host_ebpf,
            "--seccomp-notify",
            self.seccomp_notify,
        ]
        if self.name is not None:
            arguments.extend(["--name", self.name])
        return arguments


class ActrailRuntime:
    # 每个调用上下文独立保存作用域栈，嵌套退出后恢复外层 Runtime。
    _contexts: ClassVar[ContextVar[tuple[ActrailRuntime, ...]]] = ContextVar(
        "actrail_runtime_contexts", default=()
    )

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
        launch_options: LaunchOptions | None = None,
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
        self._launch_options = launch_options or LaunchOptions()

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
        launch_options: LaunchOptions | None = None,
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
            launch_options=launch_options,
        )

    def __enter__(self) -> Self:
        self._contexts.set((*self._contexts.get(), self))
        return self

    @classmethod
    def current(cls) -> ActrailRuntime:
        """取得当前 with 作用域中的 Runtime。"""
        contexts = cls._contexts.get()
        if not contexts:
            raise RuntimeError("No active AcTrail runtime context")
        return contexts[-1]

    def __exit__(
        self,
        exception_type: type[BaseException] | None,
        exception: BaseException | None,
        traceback: TracebackType | None,
    ) -> None:
        """Stop the daemon while preserving an exception from the test body."""
        try:
            result = self.stop()
            if result is not None and result.returncode != 0:
                raise RuntimeError(f"actraild stop failed: {result.output}")
        except Exception as error:
            if exception is None:
                raise
            exception.add_note(f"AcTrail runtime cleanup failed: {error}")
        finally:
            self._contexts.set(self._contexts.get()[:-1])

    @property
    def command_timeout_seconds(self) -> int:
        return self._command_timeout_seconds

    def start_actraild(self) -> list[CommandResult]:
        """Refresh the default configuration with patches and start the daemon."""
        return self.prepare()

    def prepare(self) -> list[CommandResult]:
        results = [
            self.run_checked(self._init_command()),
            self.run_checked([*self._daemon_command(), "stop"]),
        ]
        if self._clean_control_state:
            results.append(
                self.run_checked([*self._control_command(), "clean"])
            )
        # A failed start may already have spawned the daemon; retain cleanup ownership.
        self._started = True
        results.append(self.run_checked([*self._daemon_command(), "start"]))
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

    @contextmanager
    def launch_scope(self, options: LaunchOptions):
        """Bind this runtime and its launch options for one scenario."""
        previous = self._launch_options
        self._launch_options = options
        token = self._contexts.set((*self._contexts.get(), self))
        try:
            yield
        finally:
            self._contexts.reset(token)
            self._launch_options = previous

    @property
    def launch_options(self) -> LaunchOptions:
        """Return the options for the current launch scope."""
        return self._launch_options

    def daemon_command(self, *arguments: Path | str) -> list[Path | str]:
        """构造使用当前 Runtime 配置的 daemon 命令。"""
        return [*self._daemon_command(), *arguments]

    def launch_with_ctl(
        self,
        command: list[Path | str],
        *,
        environment: dict[str, str] | None = None,
        cwd: Path | None = None,
    ) -> LaunchResult:
        """解析 AcTrail 启动前缀后，交付进程和 trace_id。

        返回时 stdout 的下一段内容属于 agent。调用方接管进程和管道。
        """
        argv = self.control_command(
            "launch", *self._launch_options.arguments(), "--",
            *command,
        )
        process = subprocess.Popen(
            [str(argument) for argument in argv],
            cwd=cwd or self._repo,
            env=environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            encoding="utf-8",
            start_new_session=True,
        )
        # 使用同一个文本流读取启动前缀，缓冲区中后续的 agent 数据仍可读取。
        # 启动失败或超时时，进程尚未交付，由 Runtime 负责清理。
        with ThreadPoolExecutor(max_workers=1, thread_name_prefix="actrail-launch") as reader:
            pending = reader.submit(self._parse_launch_output, process)
            try:
                return pending.result(timeout=self._command_timeout_seconds)
            except BaseException as error:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
                reader.shutdown(wait=True)
                diagnostics = process.stderr.read()
                if diagnostics:
                    error.add_note(diagnostics)
                for stream in (process.stdin, process.stdout, process.stderr):
                    stream.close()
                raise

    @staticmethod
    def _parse_launch_output(process: subprocess.Popen[str]) -> LaunchResult:
        """actrailctl 在放行子进程前输出 trace 登记结果，读取到此为止。"""
        startup_output = []
        permissions = {}
        capabilities = ()
        for line in process.stdout:
            if line.startswith("deployment_required_capabilities="):
                capabilities = tuple(filter(None, line.partition("=")[2].strip().split(",")))
            for name in ("requested", "selected", "degraded"):
                prefix = f"deployment_permissions_{name}="
                if line.startswith(prefix):
                    permissions[name] = line[len(prefix):].strip()
            matched = re.fullmatch(r"trace trace-(\d+) entered Active\r?\n?", line)
            if matched:
                selected = None
                if len(permissions) == 3:
                    selected = LaunchPermissions(
                        requested=permissions["requested"],
                        selected=permissions["selected"],
                        degraded=permissions["degraded"] == "true",
                    )
                return LaunchResult(process, int(matched.group(1)), selected, capabilities)
            startup_output.append(line)
        raise RuntimeError("actrailctl exited before reporting an active trace:\n" + "".join(startup_output))

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
