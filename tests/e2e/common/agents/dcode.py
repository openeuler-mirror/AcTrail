"""Encapsulate Dcode stdin and ACP protocols with owned session cleanup."""
from __future__ import annotations

from pathlib import Path
from contextlib import ExitStack
import json
import subprocess
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from tests.e2e.common.testing_env.constants import DEFAULTS as MODEL_DEFAULTS
from .base import Agent, AgentArguments, AgentResult
from .constants import DEFAULTS
from .process import AgentProcess
from .protocols.acp import AcpSession
from tests.e2e.common.workspace import Workspace


class DcodeAgent(Agent):
    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """通过 Dcode 的有效配置查询默认模型。"""
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("DCODE_E2E_BINARY", "dcode")
        if selected is None:
            return None
        try:
            result = subprocess.run(
                [str(selected), "config", "get", "models.default", "--json"],
                env=discovery.environment(selected), capture_output=True, text=True,
                timeout=MODEL_DEFAULTS.discovery_timeout_seconds,
            )
        except (OSError, subprocess.TimeoutExpired):
            return None
        if result.returncode != 0:
            return None
        try:
            model = json.loads(result.stdout)["data"]["value"]
        except (json.JSONDecodeError, KeyError, TypeError):
            return None
        return model if isinstance(model, str) and model else None

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("DCODE_E2E_BINARY", "dcode")
        if self._binary is None:
            raise FileNotFoundError("Dcode executable not found")
        self._environment = environment if environment is not None else discovery.environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._shutdown_timeout = shutdown_timeout_seconds
        self._resources = ExitStack()
        self._session = AgentProcess(shutdown_timeout_seconds)
        self._stream_mode = False
        self._acp: AcpSession | None = None

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def launch(self, stream_mode: bool = True) -> Self:
        """直接启动用于可用性探测，同样由 Agent 负责会话和目录清理。"""
        return self._launch(self._session.launch_direct, stream_mode)

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        arguments = AgentArguments(self._arguments)
        if options is not None and options.max_turns is not None and not stream_mode:
            arguments.remove(values=("--max-turns",))
            arguments.append("--max-turns", str(options.max_turns))
        self._stream_mode = stream_mode
        try:
            owns_directory = self._cwd is None and Workspace.current() is None
            workspace = self._resources.enter_context(Workspace.scope(self._cwd))
            if owns_directory:
                # 独占目录中的 threads 属于本次运行；借用目录不执行整目录删除。
                self._resources.push(self._exit_threads)
            self._acp = AcpSession(self._session, workspace.path, protocol_version=1) if stream_mode else None
            mode = ["--acp"] if stream_mode else ["--stdin", "-q"]
            launch_process([self._binary, *arguments.values, *mode],
                           environment=self._environment, cwd=None)
        except BaseException as error:
            self.__exit__(type(error), error, error.__traceback__)
            raise
        return self

    def run(self, prompt: str) -> None:
        if self._stream_mode:
            self._session.submit(lambda: self._acp.run(prompt), single_task=False)
        else:
            self._session.submit(lambda: self._execute(prompt), single_task=True)

    def _execute(self, prompt: str) -> AgentResult:
        process = self._session.process
        process.stdin.write(prompt)
        process.stdin.close()
        output = process.stdout.read()
        code = process.wait()
        return AgentResult(output, code == 0, None if code == 0 else f"Dcode exited with {code}")

    def join_all(self, timeout: float | None, **kwargs) -> None:
        self._session.join_all(timeout)

    @property
    def results(self) -> list[AgentResult]:
        return self._session.results

    @property
    def launch_result(self) -> LaunchResult | None:
        return self._session.launch_result

    @property
    def returncode(self) -> int | None:
        return self._session.returncode

    @property
    def diagnostics(self) -> str:
        return self._session.diagnostics

    @property
    def command(self) -> tuple[str, ...]:
        return self._session.command

    def __exit__(self, exception_type, exception, traceback) -> None:
        try:
            self._session.close()
        except BaseException as error:
            if exception is None:
                self._resources.__exit__(type(error), error, error.__traceback__)
                raise
            exception.add_note(f"Dcode process cleanup failed: {error}")
        self._resources.__exit__(exception_type, exception, traceback)

    def _exit_threads(self, exception_type, exception, traceback) -> None:
        """先删除本次独占目录的会话，再由 Workspace 释放目录。"""
        try:
            listing = self._thread_command("list", "--cwd", str(Workspace.current().path), "--json")
            for thread in json.loads(listing)["data"]:
                self._thread_command("delete", thread["thread_id"], "--json")
        except Exception as error:
            if exception is None:
                raise
            exception.add_note(f"Dcode thread cleanup failed: {error}")

    def _thread_command(self, *arguments: str) -> str:
        result = subprocess.run(
            [str(self._binary), "threads", *arguments], env=self._environment,
            capture_output=True, text=True, timeout=self._shutdown_timeout, check=True,
        )
        return result.stdout
