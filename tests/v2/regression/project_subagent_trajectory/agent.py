from __future__ import annotations

import os
from abc import ABC, abstractmethod
from pathlib import Path

from tests.v2.common.testing_env import AgentBinaryDiscovery, default_claude_model


class ProjectSubagentAgent(ABC):
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

    def _task_contract(self) -> str:
        return (
            "The main agent must not call bash, shell, read, glob, grep, find, or git. "
            "Delegate these independent tasks: (1) use Bash command "
            "`find crates -mindepth 1 -maxdepth 1 -type d -printf '.\\n' | wc -l`, "
            "and report the count; (2) use Bash command `git log -1 --format=%cI`, and "
            "report the timestamp; (3) use Bash command `git branch --show-current`, "
            "and report the branch. Divide the work exactly as assigned. Each child must run "
            "only its specified query and return immediately; do not explore unrelated "
            "files, broaden the task, or inspect another child's result. Only when the "
            "specified command fails may that child make one minimal corrective attempt. "
            "After all three children complete, reply with one concise summary."
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
            "Inspect the current project using exactly three concurrent task/general "
            "subagents. Your first assistant response must contain all three task tool "
            "calls together; do not wait for one child before starting another. "
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
            "Inspect the current project by launching exactly three general-purpose "
            "Agent subagents concurrently. Start all three independent Agent calls in "
            "one assistant response before waiting for their results. "
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
            "Inspect the current project using exactly three independent subagents. "
            "Call spawn_subagent three times before calling join_subagent; the main "
            "agent must only collect and summarize their results. If spawn_subagent "
            "or join_subagent is unavailable, or any spawn/join invocation fails, do "
            "not inspect the project or complete the tasks yourself; reply only with "
            "`没有`. "
            + self._task_contract()
        )
