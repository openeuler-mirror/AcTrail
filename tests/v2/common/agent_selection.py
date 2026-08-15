from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.testing_env import AgentBinaryDiscovery, default_claude_model


@dataclass(frozen=True)
class AgentSelection:
    kind: str
    binary: Path
    environment: dict[str, str]

    def command(self, prompt: str) -> list[Path | str]:
        if self.kind == "xiaoo":
            return [
                self.binary,
                "--cli",
                "run",
                "--no-tools",
                "--max-turns",
                "1",
                "--prompt",
                prompt,
            ]
        if self.kind == "pi":
            return [self.binary, "-p", prompt, "--no-session"]
        if self.kind == "opencode":
            return [self.binary, "run", prompt]
        if self.kind == "claude":
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
                "--tools",
                "",
            ]
        if self.kind == "codex":
            return [
                self.binary,
                "exec",
                "--ephemeral",
                "-m",
                os.environ.get("CODEX_E2E_MODEL", "gpt-5.5"),
                "-c",
                "model_reasoning_effort="
                + os.environ.get("CODEX_E2E_REASONING_EFFORT", "low"),
                prompt,
            ]
        raise ValueError(f"unsupported agent kind: {self.kind}")


class AgentSelector:
    _CANDIDATES: tuple[tuple[str, str, str], ...] = (
        ("xiaoo", "XIAOO_E2E_BINARY", "xiaoo"),
        ("pi", "PI_E2E_BINARY", "pi"),
        ("opencode", "OPENCODE_E2E_BINARY", "opencode"),
        ("claude", "CLAUDE_E2E_BINARY", "claude"),
        ("codex", "CODEX_E2E_BINARY", "codex"),
    )

    def __init__(self, repo: Path):
        self._discovery = AgentBinaryDiscovery(repo)

    def select(
        self,
        test_context: TestingContextSingleton,
        *,
        kinds: tuple[str, ...] | None = None,
    ) -> AgentSelection | None:
        probe_all = _probe_all_agents_enabled()
        candidates = self._CANDIDATES
        if kinds is not None:
            by_kind = {candidate[0]: candidate for candidate in candidates}
            candidates = tuple(by_kind[kind] for kind in kinds if kind in by_kind)
        for kind, variable, executable in candidates:
            binary = self._discovery.resolve(variable, executable)
            if binary is None:
                continue
            environment = self._discovery.environment(binary)
            if probe_all:
                test_context.report_progress(
                    "agent_availability",
                    f"checking {kind} availability",
                )
                if test_context.check_agent_availability(
                    kind,
                    binary,
                    environment,
                ):
                    return AgentSelection(kind, binary, environment)
                continue
            test_context.report_progress(
                "agent_selection",
                f"selecting first available agent: {kind}",
            )
            return AgentSelection(kind, binary, environment)
        return None


def _probe_all_agents_enabled() -> bool:
    """Whether selection should probe every found agent until one passes.

    Default is false: the first executable agent binary wins without a real
    model call. Set ACTRAIL_TEST_AGENT_PROBE_ALL=1 to restore per-candidate
    availability probing.
    """
    return os.environ.get("ACTRAIL_TEST_AGENT_PROBE_ALL", "").strip().lower() in {
        "1",
        "true",
        "yes",
        "on",
    }
