"""Console output helpers for the overall benchmark (TTY-aware coloring)."""

from __future__ import annotations

import sys
from typing import TextIO

from scenario.model import ScenarioMeta


_ANSI_COLORS = {
    "cyan": "\033[36m",
    "green": "\033[32m",
    "reset": "\033[0m",
}


def colorize(
    text: str,
    color: str,
    stream: TextIO = sys.stdout,
) -> str:
    if not stream.isatty():
        return text
    return f"{_ANSI_COLORS[color]}{text}{_ANSI_COLORS['reset']}"


def print_scenario_list(
    scenarios: tuple[ScenarioMeta, ...],
    *,
    stream: TextIO = sys.stdout,
) -> None:
    print(colorize("available scenarios", "cyan", stream), file=stream)
    for scenario in scenarios:
        print(
            f"  {colorize(scenario.scenario_id, 'green', stream)}",
            file=stream,
        )
        meta = _meta_text(scenario)
        if meta:
            print(f"    {meta}", file=stream)
        print(f"    {scenario.description}", file=stream)


def _meta_text(scenario: ScenarioMeta) -> str:
    if scenario.rounds is None and not scenario.generator_type:
        return ""

    def fmt(value: int | None) -> str:
        return "inf" if value is None else str(value)

    parts = [
        f"rounds={fmt(scenario.rounds)}",
        (
            f"(tool={fmt(scenario.tool_rounds)}, "
            f"message={fmt(scenario.message_rounds)})"
        ),
    ]
    if scenario.tools:
        parts.append(f"tools=[{','.join(scenario.tools)}]")
    return " ".join(parts)
