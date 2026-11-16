from __future__ import annotations

from dataclasses import dataclass


@dataclass(frozen=True)
class AgentDefaults:
    availability_timeout_seconds: int = 60
    discovery_timeout_seconds: int = 30
    availability_prompt: str = "directly answer hello"
    codex_model: str = "gpt-5.5"
    codex_reasoning_effort: str = "low"
    claude_model: str = "sonnet"
    xiaoo_max_turns: int = 1
    probe_all: bool = False
    native_binary_override: bool = True
    candidates: tuple[tuple[str, str, str], ...] = (
        ("xiaoo", "XIAOO_E2E_BINARY", "xiaoo"),
        ("pi", "PI_E2E_BINARY", "pi"),
        ("opencode", "OPENCODE_E2E_BINARY", "opencode"),
        ("claude", "CLAUDE_E2E_BINARY", "claude"),
        ("codex", "CODEX_E2E_BINARY", "codex"),
    )


DEFAULTS = AgentDefaults()
