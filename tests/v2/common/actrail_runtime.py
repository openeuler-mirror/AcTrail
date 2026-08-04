from __future__ import annotations

import json
import subprocess
from dataclasses import dataclass
from pathlib import Path

from .output import TestOutput


@dataclass(frozen=True)
class CommandResult:
    argv: tuple[str, ...]
    returncode: int
    stdout: str
    stderr: str

    @property
    def output(self) -> str:
        return self.stdout + self.stderr


class ActrailRuntime:
    def __init__(
        self,
        repo: Path,
        bin_dir: Path,
        command_timeout_seconds: int,
        output: TestOutput,
        operator_config: Path | None = None,
        operator_config_patch: Path | None = None,
    ):
        self._repo = repo
        self._bin_dir = bin_dir if bin_dir.is_absolute() else repo / bin_dir
        self._command_timeout_seconds = command_timeout_seconds
        self._output = output
        self._operator_config = operator_config
        self._operator_config_patch = operator_config_patch
        self.actraild = self._require_binary("actraild")
        self.actrailctl = self._require_binary("actrailctl")
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
    ) -> "ActrailRuntime":
        work_dir = work_dir.resolve()
        if not work_dir.is_dir():
            raise RuntimeError(
                f"isolated AcTrail work directory is missing: {work_dir}"
            )
        operator_config = work_dir / "actraild.conf"
        operator_config_patch = work_dir / "actraild.patch.toml"
        cls._write_isolated_operator_config_patch(
            operator_config_patch,
            work_dir,
        )
        return cls(
            repo,
            bin_dir,
            command_timeout_seconds,
            output,
            operator_config,
            operator_config_patch,
        )

    def prepare(self) -> list[CommandResult]:
        results = [
            self.run_checked(self._init_command()),
            self.run_checked([*self._daemon_command(), "stop"]),
            self.run_checked([*self._control_command(), "clean"]),
            self.run_checked([*self._daemon_command(), "start"]),
        ]
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
    ) -> CommandResult:
        command = tuple(str(argument) for argument in argv)
        completed = subprocess.run(
            command,
            cwd=self._repo,
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
        command: list[Path | str] = [self.actrailctl]
        if self._operator_config is not None:
            command.extend(["--config", self._operator_config])
        return command

    @staticmethod
    def _write_isolated_operator_config_patch(
        path: Path,
        work_dir: Path,
    ) -> None:
        quoted = {
            name: json.dumps(str(work_dir / relative))
            for name, relative in {
                "socket": "run/control.sock",
                "pid": "run/actraild.pid",
                "log": "log/actraild.log",
                "storage": "data/actrail.sqlite",
                "export": "data/export",
                "tls_sync": "run/tls-sync.sock",
                "cluster_spool": "data/cluster-spool",
                "cluster_state": "data/cluster-report-state.sqlite",
                "cluster_root": "data/cluster",
                "plugins": "plugins",
            }.items()
        }
        path.write_text(
            "[control]\n"
            f"socket_path = {quoted['socket']}\n"
            f"pid_file = {quoted['pid']}\n"
            f"log_path = {quoted['log']}\n"
            "\n[storage.sqlite]\n"
            f"path = {quoted['storage']}\n"
            "\n[storage.retention]\n"
            "enabled = false\n"
            "\n[export.snapshot]\n"
            f"directory = {quoted['export']}\n"
            "\n[payload.tls]\n"
            f"sync_event_socket_path = {quoted['tls_sync']}\n"
            "\n[cluster.report]\n"
            f"spool_dir = {quoted['cluster_spool']}\n"
            f"state_path = {quoted['cluster_state']}\n"
            "\n[cluster.center]\n"
            f"root_dir = {quoted['cluster_root']}\n"
            "\n[plugins.discovery]\n"
            f"directory = {quoted['plugins']}\n",
            encoding="utf-8",
        )
