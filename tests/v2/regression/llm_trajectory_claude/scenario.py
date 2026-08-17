from __future__ import annotations

import re
import secrets
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.regression.llm_trajectory.scenario import FixtureRepository


class ClaudeSubagentScenario:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")

    def __init__(
        self,
        runtime: ActrailRuntime,
        binary: Path,
        environment: dict[str, str],
        fixture: FixtureRepository,
        model: str,
        launch_timeout_seconds: int,
    ):
        self._runtime = runtime
        self._binary = binary.resolve()
        self._environment = environment
        self._fixture = fixture
        self._model = model
        self._launch_timeout_seconds = launch_timeout_seconds
        self.trace_name = f"CLAUDE_LLM_TRAJECTORY_{secrets.token_hex(8)}"
        self.task_marker = f"CLAUDE_SUBTASK_{secrets.token_hex(8)}"
        self.answer_marker = f"CLAUDE_RESULT_{secrets.token_hex(8)}"

    def run(self) -> tuple[int, CommandResult]:
        result = self._runtime.run(
            [
                *self._runtime.control_command("launch"),
                "--name",
                self.trace_name,
                "--host-ebpf",
                "required",
                "--seccomp-notify",
                "auto",
                "--",
                self._binary,
                self._prompt(),
                "--print",
                "--output-format",
                "text",
                "--model",
                self._model,
                "--no-session-persistence",
                "--safe-mode",
                "--permission-mode",
                "dontAsk",
                "--prompt-suggestions",
                "false",
                "--tools",
                "Agent,Bash",
                "--allowedTools",
                "Agent,Bash(git rev-parse HEAD)",
            ],
            timeout_seconds=self._launch_timeout_seconds,
            environment=self._environment,
            cwd=self._fixture.path,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"Claude launch exited with {result.returncode}: "
                f"{result.output[-6000:]}"
            )
        trace_ids = [int(value) for value in self._TRACE_PATTERN.findall(result.output)]
        if len(trace_ids) != 1:
            raise RuntimeError(
                f"Claude launch reported unexpected trace ids {trace_ids}: "
                f"{result.output[-6000:]}"
            )
        return trace_ids[0], result

    def _prompt(self) -> str:
        return (
            "You must use the Agent tool exactly once with subagent_type "
            "general-purpose. The main agent must not use Bash or inspect Git "
            "itself. Include this exact token in the delegated prompt: "
            f"{self.task_marker}. Tell the subagent to execute exactly "
            "`git rev-parse HEAD` using Bash in the current repository and return "
            "the complete commit id. Wait for the Agent result. Then reply with "
            f"exactly `{self.answer_marker} <commit-id>`."
        )
