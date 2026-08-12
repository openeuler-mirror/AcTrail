"""Round statistics for scenario generators (used to build scenario meta)."""

from __future__ import annotations

from dataclasses import dataclass

from ..model import ToolCall
from .interface import (
    GenerationOptions,
    GenerationUnavailable,
    ScenarioGenerator,
)


@dataclass(frozen=True, slots=True)
class RoundStats:
    rounds: int | None          # None = infinite
    tool_rounds: int | None     # None = infinite
    message_rounds: int | None  # None = infinite
    tools: tuple[str, ...] = ()


class _AcceptAll:
    def accepts(self, call: ToolCall) -> bool:
        return True


def scenario_round_stats(
    generator: ScenarioGenerator,
    *,
    cap: int = 100_000,
) -> RoundStats:
    """Count tool/message rounds by bounded dry-run with all tools accepted.

    ``None`` marks an infinite dimension (the run hit ``cap``). The tool name
    set is exact for what the generator declares, regardless of the cap.
    """

    options = GenerationOptions(tool_calls=_AcceptAll(), declares_tools=True)
    execution = generator.reset()
    tool_rounds = 0
    message_rounds = 0
    tool_names: set[str] = set()
    for _ in range(cap):
        try:
            result = execution.next(options)
        except StopIteration:
            break
        if isinstance(result, GenerationUnavailable):
            continue
        has_tool = False
        for block in result.response.blocks:
            call = block.tool_call
            if call is not None:
                has_tool = True
                tool_names.add(call.name)
        if has_tool:
            tool_rounds += 1
        else:
            message_rounds += 1
    else:
        return RoundStats(
            rounds=None,
            tool_rounds=None if tool_rounds else 0,
            message_rounds=None if message_rounds else 0,
            tools=tuple(sorted(tool_names)),
        )
    return RoundStats(
        rounds=tool_rounds + message_rounds,
        tool_rounds=tool_rounds,
        message_rounds=message_rounds,
        tools=tuple(sorted(tool_names)),
    )
