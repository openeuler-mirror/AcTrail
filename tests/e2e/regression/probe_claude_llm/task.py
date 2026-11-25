"""Run a real Claude task through the shared Agent and AcTrail lifecycle."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.actrail_runtime import CommandResult
from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError
from tests.e2e.common.testing_env import AgentBinaryDiscovery

from .config import ProbeClaudeLLMConfig

from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class ProbeClaudeLLMResult(CommandResult):
    """Claude 任务输出及启动时取得的 trace_id。"""

    trace_id: int


class ProbeClaudeLLMTask:
    def __init__(self, config: ProbeClaudeLLMConfig):
        self._config = config
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._claude = self._resolve_claude()

    def run(self) -> ProbeClaudeLLMResult:
        agent = AgentFactory.claude(
            binary=self._claude,
            arguments=("--model", self._config.model, "--tools", ""),
            environment=self.environment(),
        )
        with agent.launchWithAcTrailCtl():
            agent.run(CASE_CONSTANTS.prompt)
            agent.join_all(self._config.launch_timeout_seconds)
            result = agent.results[0]

        stderr = agent.diagnostics
        if result.error:
            stderr += f"\n{result.error}\n"
        return ProbeClaudeLLMResult(
            argv=agent.command,
            returncode=agent.returncode or int(not result.succeeded),
            stdout=result.output,
            stderr=stderr,
            trace_id=agent.launch_result.trace_id,
        )

    @property
    def binary(self) -> Path:
        return self._claude

    def _resolve_claude(self) -> Path:
        configured = self._config.claude_binary
        if configured is not None:
            if AgentBinaryDiscovery.is_executable(configured):
                return configured
            raise AgentBinaryNotFoundError(
                "configured Claude executable is unavailable: "
                f"{configured}"
            )
        binary = self._discovery.resolve("CLAUDE_E2E_BINARY", "claude")
        if binary is None:
            raise AgentBinaryNotFoundError(
                "Claude executable not found; set CLAUDE_E2E_BINARY to its path"
            )
        return binary

    def environment(self) -> dict[str, str]:
        return self._discovery.environment(self._claude)
