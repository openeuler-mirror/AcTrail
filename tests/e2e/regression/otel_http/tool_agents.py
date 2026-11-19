"""Supported agents with their tools enabled, and a task that needs two tool calls."""

from __future__ import annotations

import os
import secrets
from dataclasses import dataclass
from pathlib import Path

from tests.e2e.common.testing_env import AgentBinaryDiscovery
from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.testing_env.constants import DEFAULTS

from .constants import CASE_CONSTANTS

#: (agent kind, binary variable, executable), in the order the round runs them.
TOOL_AGENTS = (
    ("xiaoo", "XIAOO_E2E_BINARY", "xiaoo"),
    ("pi", "PI_E2E_BINARY", "pi"),
    ("opencode", "OPENCODE_E2E_BINARY", "opencode"),
    ("claude", "CLAUDE_E2E_BINARY", "claude"),
    ("codex", "CODEX_E2E_BINARY", "codex"),
    ("qodercli", "QODERCLI_E2E_BINARY", "qodercli"),
    ("dcode", "DCODE_E2E_BINARY", "dcode"),
    ("grok", "GROK_E2E_BINARY", "grok"),
)


@dataclass(frozen=True)
class ToolChainTask:
    """Two files where only the first names the second, so the agent reads them in turn."""

    first: Path

    @classmethod
    def create(cls, directory: Path) -> "ToolChainTask":
        directory.mkdir(parents=True)
        token = secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)
        answer = f"TOOL_CHAIN_{token}"
        second = directory / f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}.txt"
        second.write_text(f"{answer}\n", encoding="utf-8")
        first = directory / "start.txt"
        first.write_text(f"The answer is in the file {second}\n", encoding="utf-8")
        return cls(first)

    @property
    def prompt(self) -> str:
        return (
            f"Use a tool to read the file {self.first}. It names another file; "
            "use a tool to read that file too, then describe what you found."
        )


@dataclass(frozen=True)
class ToolAgent:
    kind: str
    binary: Path
    environment: dict[str, str]
    model: str | None = None

    def command(self, prompt: str) -> list[Path | str]:
        turns = str(CASE_CONSTANTS.tool_agent_max_turns)
        if self.kind == "xiaoo":
            return [self.binary, "--cli", "run", "--max-turns", turns, "--prompt", prompt]
        if self.kind == "pi":
            return [self.binary, "-p", prompt, "--no-session", "--tools", "read,bash"]
        if self.kind == "opencode":
            return [self.binary, "run", prompt]
        if self.kind == "claude":
            return [
                self.binary,
                "-p",
                prompt,
                "--model",
                AgentFactory.claude.get_default_llm_model(),
                "--no-session-persistence",
                "--permission-mode",
                "dontAsk",
                "--allowedTools",
                "Read,Bash",
            ]
        if self.kind == "codex":
            return [
                self.binary,
                "exec",
                "--ephemeral",
                "--skip-git-repo-check",
                "--sandbox",
                "read-only",
                "-m",
                self.model or DEFAULTS.codex_model,
                "-c",
                "model_reasoning_effort="
                + os.environ.get("CODEX_E2E_REASONING_EFFORT", DEFAULTS.codex_reasoning_effort),
                prompt,
            ]
        if self.kind == "qodercli":
            return [
                self.binary,
                "-p",
                prompt,
                "--no-session-persistence",
                "--permission-mode",
                "dont_ask",
                "--allowed-tools",
                "Read",
                "--allowed-tools",
                "Bash",
            ]
        if self.kind == "dcode":
            return [self.binary, "-n", prompt, "-q", "--no-mcp"]
        if self.kind == "grok":
            return [
                self.binary,
                "-p",
                prompt,
                "--always-approve",
                "--no-subagents",
                "--disable-web-search",
                "--max-turns",
                turns,
            ]
        raise ValueError(f"unsupported agent kind: {self.kind}")


def resolve_tool_agent(
    discovery: AgentBinaryDiscovery,
    kind: str,
    variable: str,
    executable: str,
) -> ToolAgent | None:
    binary = discovery.resolve(variable, executable)
    if binary is None:
        return None
    if kind != "codex":
        return ToolAgent(kind, binary, discovery.environment(binary))
    model = AgentFactory.codex.get_default_llm_model(
        binary
    )
    return ToolAgent(kind, binary, discovery.codex_environment(binary), model)
