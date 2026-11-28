"""Run a Claude session with one context compaction between two tasks."""
from __future__ import annotations

from pathlib import Path

from tests.e2e.common.agents import Agent, AgentFactory
from tests.e2e.common.runner.case_scope import CaseScope
from .constants import DEFAULTS


class CompactionTask:
    def __init__(self, timeout: float):
        self._timeout = timeout

    def run(self, case: CaseScope, binary: Path) -> int:
        with AgentFactory.claude(binary=binary).launchWithAcTrailCtl(stream_mode=True) as agent:
            trace_id = agent.launch_result.trace_id
            with case.step("Build context for compaction"):
                agent.run(DEFAULTS.seed_prompt)
                self._require_result(agent)
                agent.run(DEFAULTS.followup_prompt)
                self._require_result(agent)
            with case.step("Compact the context"):
                agent.compact()
                self._require_result(agent)
            with case.step("Send a request after compaction"):
                agent.run(DEFAULTS.post_compaction_prompt)
                self._require_result(agent)
        if agent.returncode != 0:
            raise AssertionError(f"Claude launch exited with {agent.returncode}")
        return trace_id

    def _require_result(self, agent: Agent) -> None:
        """统一检查普通任务与压缩任务的结果，不依赖具体事件格式。"""
        agent.join_all(self._timeout)
        result = agent.results[-1]
        if not result.succeeded:
            raise AssertionError(result.error or "Claude task failed")
