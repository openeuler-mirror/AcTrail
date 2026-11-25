"""Encapsulate OpenCode's single-task CLI and ACP conversation protocols."""
from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from tests.e2e.common.testing_env.constants import DEFAULTS as MODEL_DEFAULTS
from tests.e2e.common.workspace import Workspace
from .base import Agent, AgentResult
from .constants import DEFAULTS
from .process import AgentProcess
from .protocols.acp import AcpSession


class OpenCodeAgent(Agent):
    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """读取 OpenCode 合并后的有效配置，保留显式测试模型覆盖。"""
        configured = os.environ.get("OPENCODE_E2E_MODEL")
        if configured:
            return configured
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("OPENCODE_E2E_BINARY", "opencode")
        if selected is None:
            return None
        workspace = Workspace.current()
        try:
            result = subprocess.run(
                [str(selected), "debug", "config"], env=discovery.environment(selected),
                cwd=workspace.path if workspace is not None else None,
                capture_output=True, text=True, timeout=MODEL_DEFAULTS.discovery_timeout_seconds,
            )
        except (OSError, subprocess.TimeoutExpired):
            return None
        if result.returncode != 0:
            return None
        try:
            model = json.loads(result.stdout).get("model")
        except (json.JSONDecodeError, AttributeError):
            return None
        return model if isinstance(model, str) and model else None

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("OPENCODE_E2E_BINARY", "opencode")
        if self._binary is None:
            raise FileNotFoundError("OpenCode executable not found")
        self._environment = environment if environment is not None else discovery.environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._session = AgentProcess(shutdown_timeout_seconds)
        self._stream_mode = False

    def launch(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch_direct, stream_mode)

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        self._stream_mode = stream_mode
        mode = ["acp", *self._arguments] if stream_mode else [
            "run", *self._arguments, "--format", "json",
        ]
        launch_process(
            [self._binary, *mode],
            environment=self._environment, cwd=self._cwd,
        )
        self._acp = AcpSession(self._session, Workspace.current().path, protocol_version=1)
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
        output = []
        failure = None
        for line in process.stdout:
            event = json.loads(line)
            if event.get("type") == "text":
                output.append(event.get("part", {}).get("text", ""))
            elif event.get("type") == "error":
                failure = json.dumps(event.get("error"), ensure_ascii=False)
        code = process.wait()
        if code:
            failure = failure or f"OpenCode exited with {code}"
        return AgentResult("\n".join(output), failure is None, failure)

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
        self._session.__exit__(exception_type, exception, traceback)
        # TODO: 取得新建会话的明确所有权后提供清理，输出 sessionID 可能是复用会话。
