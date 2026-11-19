"""Probe real agents through their shared lifecycle and cache availability."""
from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path
from typing import Mapping

from tests.e2e.common.agents import AgentFactory

from .constants import DEFAULTS


class AgentAvailability:
    def __init__(self):
        self._agent2availability: dict[tuple[str, str], bool] = {}
        self._timeout_seconds = int(
            os.environ.get("ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS", str(DEFAULTS.availability_timeout_seconds))
        )

    def check_agent_availability(
        self,
        agent_name: str,
        binary: Path | str | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> bool:
        normalized = agent_name.strip().lower()
        selected_binary = str(binary or normalized)
        cache_key = (normalized, selected_binary)
        if cache_key not in self._agent2availability:
            self._agent2availability[cache_key] = self._update_agent_availability(
                normalized, selected_binary, environment,
            )
        return self._agent2availability[cache_key]

    def _update_agent_availability(
        self,
        agent_name: str,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        try:
            agent_type = getattr(AgentFactory, agent_name)
        except AttributeError:
            raise ValueError(f"unknown agent availability target: {agent_name}") from None
        selected_environment = os.environ.copy() if environment is None else dict(environment)
        executable = shutil.which(binary, path=selected_environment.get("PATH"))
        if executable is None:
            return False
        selected_binary = Path(executable).resolve()
        try:
            with agent_type(
                binary=selected_binary,
                environment=selected_environment,
                arguments=self._arguments(agent_name),
                shutdown_timeout_seconds=self._timeout_seconds,
            ).launch(stream_mode=agent_name == "grok") as agent:
                agent.run(self._prompt())
                agent.join_all(timeout=self._timeout_seconds)
                result = agent.results[-1]
                succeeded = result.succeeded
            # 退出作用域完成进程与会话清理后，才返回可缓存的探测结果。
            return succeeded
        except (OSError, TimeoutError, subprocess.SubprocessError, RuntimeError):
            return False

    def _prompt(self) -> str:
        return os.environ.get(
            "ACTRAIL_TEST_AGENT_AVAILABILITY_PROMPT", DEFAULTS.availability_prompt,
        )

    def _arguments(self, agent_name: str) -> tuple[str, ...]:
        """探测场景选择模型和工具策略，参数原样交给具体 Agent。"""
        if agent_name == "codex":
            return ("--skip-git-repo-check",)
        if agent_name == "claude":
            return ("--permission-mode", "dontAsk", "--tools", "")
        if agent_name == "qodercli":
            return ("--tools", "")
        if agent_name == "xiaoo":
            return ("--no-tools", "--max-turns", str(DEFAULTS.xiaoo_max_turns))
        if agent_name == "dcode":
            return ("--no-mcp", "--timeout", str(self._timeout_seconds))
        return ()

