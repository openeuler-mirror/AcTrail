from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, Iterator

from ...model import (
    ResponseTemplate,
    ScenarioConfigurationError,
)
from ..interface import (
    GenerationOptions,
    GenerationUnavailable,
    GeneratorParameters,
    ScenarioGenerator,
)


@dataclass(frozen=True, slots=True)
class RecordedGenerator(ScenarioGenerator):
    """Replay a recorded session streamed from two JSONL round files.

    Requests that declare tools consume the next compatible round from the
    tool file; requests without tools consume the next message round. Tool
    rounds the client cannot serve are skipped, and once the tool queue is
    exhausted tool-declaring requests fall back to the message queue so
    agents that always declare tools (e.g. opencode) can still receive the
    recorded final answer instead of a 409. When the message queue is
    exhausted it loops from the start by default
    (``loop_exhausted_messages``).

    Rounds are parsed lazily from the JSONL files, one line per response, so
    selecting the scenario does not load the whole sequence into memory; the
    message loop re-opens the file when exhausted.
    """

    tool_source: Path
    message_source: Path
    node_parser: Callable[[object, str], ResponseTemplate]
    loop_exhausted_messages: bool = True
    lazy_load_size: int = 0

    @property
    def kind(self) -> str:
        return "recorded"

    @property
    def is_infinite(self) -> bool:
        return False

    def generate(
        self, parameters: GeneratorParameters
    ) -> Iterator[ResponseTemplate | GenerationUnavailable]:
        tool_rounds = self._load_rounds(self.tool_source)
        message_rounds = self._load_rounds(self.message_source)
        while True:
            options = parameters.options
            declares_tools = bool(
                getattr(options, "declares_tools", False)
            )
            if declares_tools:
                template = self._next_compatible(
                    tool_rounds,
                    getattr(options, "tool_calls", None),
                )
                if template is not None:
                    yield template
                    continue
            try:
                message = next(message_rounds)
            except StopIteration:
                message = None
                if (
                    self.loop_exhausted_messages
                    and self._has_rounds(self.message_source)
                ):
                    message_rounds = self._load_rounds(
                        self.message_source
                    )
                    try:
                        message = next(message_rounds)
                    except StopIteration:
                        message = None
            if message is not None:
                yield message
                continue
            if declares_tools:
                yield GenerationUnavailable(
                    "recorded tool responses exhausted"
                )
            else:
                yield GenerationUnavailable(
                    "recorded message responses exhausted"
                )

    def _load_rounds(
        self,
        source: Path,
    ) -> Iterator[ResponseTemplate]:
        nodes = self._iter_nodes(source)
        if self.lazy_load_size <= 0:
            return iter(tuple(nodes))
        return nodes

    def _iter_nodes(self, source: Path) -> Iterator[ResponseTemplate]:
        batch_size = self.lazy_load_size or 1
        index = 0
        with source.open("r", encoding="utf-8") as round_file:
            while True:
                batch = [
                    round_file.readline()
                    for _ in range(batch_size)
                ]
                if not any(batch):
                    break
                for line in batch:
                    if not line.strip():
                        continue
                    try:
                        node = json.loads(line)
                    except ValueError as error:
                        raise ScenarioConfigurationError(
                            f"invalid recorded round in {source}: {error}"
                        ) from error
                    yield self.node_parser(node, f"$[{index}]")
                    index += 1

    @classmethod
    def _next_compatible(
        cls,
        tool_rounds: Iterator[ResponseTemplate],
        tool_filter: object,
    ) -> ResponseTemplate | None:
        for template in tool_rounds:
            if cls._compatible(template, tool_filter):
                return template
        return None

    @staticmethod
    def _compatible(
        template: ResponseTemplate,
        tool_filter: object,
    ) -> bool:
        if tool_filter is None:
            return True
        for block in template.response.blocks:
            call = block.tool_call
            if call is not None and not tool_filter.accepts(call):
                return False
        return True

    @staticmethod
    def _has_rounds(source: Path) -> bool:
        try:
            return source.stat().st_size > 0
        except OSError:
            return False

    def is_eligible(self, options: GenerationOptions) -> bool:
        return True
