"""Encapsulate XiaoO's single-task CLI input and result protocol."""
from __future__ import annotations

import json
import tomllib
import os
from pathlib import Path
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult, ToolAccess
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .base import Agent, AgentArguments, AgentResult
from .constants import DEFAULTS
from .process import AgentProcess


class XiaoOAgent(Agent):
    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """从所选 XiaoO 使用的配置文件读取默认模型。"""
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("XIAOO_E2E_BINARY", "xiaoo")
        environment = discovery.environment(selected) if selected is not None else os.environ
        path = Path(environment.get("XIAOO_CONFIG") or Path(environment.get("HOME", str(Path.home()))) / ".config" / "xiaoo" / "config.toml")
        try:
            with path.open("rb") as stream:
                return tomllib.load(stream)["llm"]["model"]
        except (OSError, tomllib.TOMLDecodeError, KeyError, TypeError):
            return None

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("XIAOO_E2E_BINARY", "xiaoo")
        if self._binary is None:
            raise FileNotFoundError("XiaoO executable not found")
        self._environment = environment if environment is not None else discovery.environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._session = AgentProcess(shutdown_timeout_seconds)

    def launch(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch_direct, stream_mode)

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def _configured_arguments(self, options: LaunchOptions | None) -> list[str]:
        arguments = AgentArguments(self._arguments)
        if options is None:
            return arguments.values
        if options.tool_access is not None:
            arguments.remove(values=("--tools",), switches=("--no-tools",))
            if options.tool_access is ToolAccess.NONE:
                arguments.append("--no-tools")
            elif options.tool_access is ToolAccess.READ_ONLY:
                arguments.append("--tools", "file_read,glob,grep")
        if options.max_turns is not None:
            arguments.remove(values=("--max-turns",))
            arguments.append("--max-turns", str(options.max_turns))
        return arguments.values

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        arguments = self._configured_arguments(options)
        if stream_mode:
            raise NotImplementedError("XiaoO CLI does not support streaming input")
        launch_process(
            [self._binary, "--cli", "run", *arguments, "--format", "json"],
            environment=self._environment, cwd=self._cwd,
        )
        return self

    def run(self, prompt: str) -> None:
        self._session.submit(lambda: self._execute(prompt), single_task=True)

    def _execute(self, prompt: str) -> AgentResult:
        process = self._session.process
        # run 从 stdin 读取完整输入，EOF 才开始执行。
        process.stdin.write(prompt)
        process.stdin.close()
        result = None
        for line in process.stdout:
            event = json.loads(line)
            data = event.get("data", {})
            if event.get("type") == "response":
                result = AgentResult(data.get("raw_reply", ""), True)
            elif event.get("type") == "error":
                result = AgentResult("", False, data.get("message", "XiaoO task failed"))
        returncode = process.wait()
        if returncode:
            return AgentResult("" if result is None else result.output, False,
                               f"XiaoO exited with {returncode}: "
                               + (result.error or "" if result is not None else ""))
        if result is None:
            raise RuntimeError("XiaoO exited without a final response")
        return result

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
        """CLI 的诊断输出；退出作用域后读取可获得完整内容。"""
        return self._session.diagnostics

    @property
    def command(self) -> tuple[str, ...]:
        return self._session.command

    def __exit__(self, exception_type, exception, traceback) -> None:
        self._session.__exit__(exception_type, exception, traceback)
