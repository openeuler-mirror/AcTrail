from __future__ import annotations

import os
from abc import ABC, abstractmethod
from pathlib import Path

from tests.v2.common.testing_env import AgentBinaryDiscovery, default_claude_model


class ProjectSubagentAgent(ABC):
    DEFAULT_CANDIDATES = ("opencode", "claude", "xiaoo")

    def __init__(
        self,
        binary: Path,
        environment: dict[str, str],
    ):
        self.binary = binary.resolve()
        self.environment = environment

    @property
    @abstractmethod
    def name(self) -> str:
        raise NotImplementedError

    @abstractmethod
    def launch_argv(self, repository: Path, prompt: str) -> list[Path | str]:
        raise NotImplementedError

    @abstractmethod
    def prompt(self) -> str:
        raise NotImplementedError

    def runtime_cwd(self, repository: Path) -> Path | None:
        return None

    @classmethod
    def resolve(
        cls,
        agent_binary: str,
        discovery: AgentBinaryDiscovery,
    ) -> "ProjectSubagentAgent | None":
        implementations = {
            "opencode": OpenCodeProjectSubagentAgent,
            "claude": ClaudeProjectSubagentAgent,
            "xiaoo": XiaooProjectSubagentAgent,
        }
        implementation = implementations.get(agent_binary)
        if implementation is None:
            raise ValueError(
                "agent_binary must be one of: " + ", ".join(implementations)
            )
        binary = discovery.resolve(
            implementation.binary_environment_variable,
            implementation.executable_name,
        )
        if binary is None:
            return None
        return implementation(binary, discovery.environment(binary))

    @classmethod
    def candidates(cls, configured: str | None) -> tuple[str, ...]:
        if configured is not None:
            return (configured,)
        return cls.DEFAULT_CANDIDATES

    def _task_contract(self) -> str:
        return (
            "现在派生2个subagent，一个编写一下冒泡排序，一个编写一个二分排序。"
            "然后主agent测试哪个速度快。 "
            "Delegate exactly two independent implementation tasks: (1) one child writes "
            "`bubble_sort.py` with a tested bubble_sort function; (2) one child writes "
            "`binary_insertion_sort.py` with a tested binary_insertion_sort function. "
            "Start both children before waiting for either result. After both finish, the "
            "main agent must read both files, write `benchmark.py`, run a fair benchmark "
            "on identical deterministic integer inputs of at least three sizes, and report "
            "which implementation is faster. The main agent must perform the benchmark "
            "itself and must not rewrite either child's implementation. Keep all work in "
            "the current directory and do not inspect the AcTrail source repository."
        )


class OpenCodeProjectSubagentAgent(ProjectSubagentAgent):
    binary_environment_variable = "OPENCODE_E2E_BINARY"
    executable_name = "opencode"

    @property
    def name(self) -> str:
        return "opencode"

    def launch_argv(self, repository: Path, prompt: str) -> list[Path | str]:
        argv: list[Path | str] = [
            self.binary,
            "run",
            "--pure",
            "--auto",
            "--format",
            "json",
            "--dir",
            repository,
        ]
        model = os.environ.get("OPENCODE_E2E_MODEL", "").strip()
        if model:
            argv.extend(("--model", model))
        argv.append(prompt)
        return argv

    def prompt(self) -> str:
        return (
            "Use exactly two concurrent task/general subagents for this sorting benchmark. "
            "Your first assistant response must contain both task tool calls together. "
            + self._task_contract()
        )


class ClaudeProjectSubagentAgent(ProjectSubagentAgent):
    binary_environment_variable = "CLAUDE_E2E_BINARY"
    executable_name = "claude"

    @property
    def name(self) -> str:
        return "claude"

    def launch_argv(self, repository: Path, prompt: str) -> list[Path | str]:
        return [
            self.binary,
            prompt,
            "--print",
            "--output-format",
            "text",
            "--model",
            default_claude_model(),
            "--no-session-persistence",
            "--safe-mode",
            "--permission-mode",
            "dontAsk",
            "--prompt-suggestions",
            "false",
            "--tools",
            "Agent,Bash",
            "--allowedTools",
            "Agent,Bash",
        ]

    def runtime_cwd(self, repository: Path) -> Path | None:
        return repository

    def prompt(self) -> str:
        return (
            "Run this sorting benchmark by launching exactly two general-purpose Agent "
            "subagents concurrently. Start both independent Agent calls in one assistant "
            "response before waiting for their results. "
            + self._task_contract()
        )


class XiaooProjectSubagentAgent(ProjectSubagentAgent):
    binary_environment_variable = "XIAOO_E2E_BINARY"
    executable_name = "xiaoo"

    @property
    def name(self) -> str:
        return "xiaoo"

    def launch_argv(self, repository: Path, prompt: str) -> list[Path | str]:
        max_turns = os.environ.get(
            "PROJECT_SUBAGENT_TRAJECTORY_E2E_XIAOO_MAX_TURNS",
            "20",
        ).strip()
        if not max_turns.isdecimal() or int(max_turns) < 1:
            raise ValueError(
                "PROJECT_SUBAGENT_TRAJECTORY_E2E_XIAOO_MAX_TURNS must be a "
                "positive integer"
            )
        return [
            self.binary,
            "--cli",
            "run",
            "--max-turns",
            max_turns,
            "--prompt",
            prompt,
        ]

    def runtime_cwd(self, repository: Path) -> Path | None:
        return repository

    def prompt(self) -> str:
        return (
            "Run this sorting benchmark using exactly two independent subagents. "
            "Call spawn_subagent twice before calling join_subagent; the main "
            "agent must wait for both results before running the benchmark. If "
            "spawn_subagent or join_subagent is unavailable, or any spawn/join "
            "invocation fails, do "
            "not implement the algorithms or benchmark yourself; reply only with "
            "`没有`. "
            + self._task_contract()
        )
