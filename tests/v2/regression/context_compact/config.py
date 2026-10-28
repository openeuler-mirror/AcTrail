from __future__ import annotations

import os
import math
from dataclasses import dataclass

from tests.v2.common.core import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class ContextCompactConfig(CommonTestConfig):
    agents: tuple[str, ...]
    mode: str
    startup_timeout_seconds: float
    turn_timeout_seconds: float
    shutdown_timeout_seconds: float
    terminal_rows: int
    terminal_columns: int

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ContextCompactConfig":
        common = CommonTestConfig.from_environment(
            inputs,
            "CONTEXT_COMPACT",
        )

        def number(name: str, default: str) -> float:
            value = float(os.environ.get(f"CONTEXT_COMPACT_E2E_{name}", default))
            if not math.isfinite(value) or value <= 0:
                raise ValueError(
                    f"CONTEXT_COMPACT_E2E_{name} must be positive"
                )
            return value

        def positive_integer(name: str, default: str) -> int:
            value = int(os.environ.get(f"CONTEXT_COMPACT_E2E_{name}", default))
            if value <= 0:
                raise ValueError(
                    f"CONTEXT_COMPACT_E2E_{name} must be positive"
                )
            return value

        supported_agents = {"opencode", "claude"}
        agents = tuple(
            value.strip()
            for value in os.environ.get(
                "CONTEXT_COMPACT_E2E_AGENTS",
                "opencode,claude",
            ).split(",")
            if value.strip()
        )
        if not agents or any(agent not in supported_agents for agent in agents):
            raise ValueError(
                "CONTEXT_COMPACT_E2E_AGENTS must list opencode and/or claude in selection order"
            )
        if len(set(agents)) != len(agents):
            raise ValueError("CONTEXT_COMPACT_E2E_AGENTS must not contain duplicates")
        mode = os.environ.get("CONTEXT_COMPACT_E2E_MODE", "inference").strip()
        if mode not in {"disabled", "inference"}:
            raise ValueError("CONTEXT_COMPACT_E2E_MODE must be disabled or inference")
        return cls(
            **common.as_kwargs(),
            agents=agents,
            mode=mode,
            startup_timeout_seconds=number("STARTUP_TIMEOUT_SECONDS", "60"),
            turn_timeout_seconds=number("TURN_TIMEOUT_SECONDS", "240"),
            shutdown_timeout_seconds=number("SHUTDOWN_TIMEOUT_SECONDS", "15"),
            terminal_rows=positive_integer("TERMINAL_ROWS", "40"),
            terminal_columns=positive_integer("TERMINAL_COLUMNS", "160"),
        )
