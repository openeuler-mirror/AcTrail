"""Run Grok's ACP agent inside the observed process tree."""
from __future__ import annotations

from pathlib import Path
import subprocess
from typing import Self

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, LaunchResult
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from tests.e2e.common.testing_env.constants import DEFAULTS as MODEL_DEFAULTS
from tests.e2e.common.workspace import Workspace
from .base import Agent, AgentResult
from .constants import DEFAULTS
from .process import AgentProcess
from .protocols.acp import AcpSession


class GrokAgent(Agent):
    @classmethod
    def get_default_llm_model(cls, binary: Path | None = None) -> str | None:
        """读取 Grok 模型列表中明确标记的默认模型。"""
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        selected = binary or discovery.resolve("GROK_E2E_BINARY", "grok")
        if selected is None:
            return None
        try:
            result = subprocess.run(
                [str(selected), "models"], env=discovery.environment(selected),
                capture_output=True, text=True, timeout=MODEL_DEFAULTS.discovery_timeout_seconds,
            )
        except (OSError, subprocess.TimeoutExpired):
            return None
        if result.returncode != 0:
            return None
        for line in result.stdout.splitlines():
            if line.startswith("Default model:"):
                return line.removeprefix("Default model:").strip() or None
        return None

    def __init__(self, binary: Path | None = None, *, arguments: tuple[str, ...] = (),
                 environment: dict[str, str] | None = None, cwd: Path | None = None,
                 shutdown_timeout_seconds: float = DEFAULTS.shutdown_timeout_seconds):
        discovery = AgentBinaryDiscovery(Path(__file__).resolve().parents[4])
        self._binary = binary or discovery.resolve("GROK_E2E_BINARY", "grok")
        if self._binary is None:
            raise FileNotFoundError("Grok executable not found")
        self._environment = environment if environment is not None else discovery.environment(self._binary)
        self._arguments = arguments
        self._cwd = cwd
        self._session = AgentProcess(shutdown_timeout_seconds)

    def launch(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch_direct, stream_mode)

    def launchWithAcTrailCtl(self, stream_mode: bool = True) -> Self:
        return self._launch(self._session.launch, stream_mode, ActrailRuntime.current().launch_options)

    def _launch(self, launch_process, stream_mode: bool, options: LaunchOptions | None = None) -> Self:
        if not stream_mode:
            # TODO: headless 只接受参数或文件输入，需要独立实现单轮生命周期。
            raise NotImplementedError("Grok adapter currently requires ACP streaming mode")
        launch_process(
            [self._binary, "agent", "--no-leader", *self._arguments, "stdio"],
            environment=self._environment, cwd=self._cwd,
        )
        self._acp = AcpSession(self._session, Workspace.current().path, protocol_version="1")
        return self

    def run(self, prompt: str) -> None:
        self._session.submit(lambda: self._acp.run(prompt))

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
