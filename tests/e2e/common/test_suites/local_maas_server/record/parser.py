from __future__ import annotations

import json
from dataclasses import dataclass
from typing import Any

from scenario.model import ResponseBlock, ToolCall
from scenario.tool_alias import ToolAliasConfig


def normalize_stop(
    protocol: str,
    stop_reason: str | None,
    has_tool_calls: bool,
) -> str:
    if protocol == "openai":
        if stop_reason in {"tool_calls", "function_call"}:
            stop = "tool_call"
        elif stop_reason == "length":
            stop = "length"
        else:
            stop = "complete"
    elif stop_reason == "tool_use":
        stop = "tool_call"
    elif stop_reason == "max_tokens":
        stop = "length"
    else:
        stop = "complete"
    if stop == "tool_call" and not has_tool_calls:
        return "complete"
    if stop == "complete" and has_tool_calls:
        return "tool_call"
    return stop


@dataclass(frozen=True, slots=True)
class RecordedResponse:
    protocol: str
    stream: bool
    model: str | None
    blocks: tuple[ResponseBlock, ...]
    stop: str
    output_tokens: int

    def to_cache_document(self) -> dict[str, object]:
        return {
            "protocol": self.protocol,
            "stream": self.stream,
            "model": self.model,
            "blocks": [
                self._render_block(block) for block in self.blocks
            ],
            "stop": self.stop,
            "output_tokens": self.output_tokens,
        }

    @staticmethod
    def _render_block(block: ResponseBlock) -> dict[str, object]:
        if block.kind == "reasoning":
            return {"type": "reasoning", "chunks": list(block.fragments)}
        if block.kind == "message":
            return {"type": "message", "text": block.text}
        call = block.tool_call
        if call is None:
            raise ValueError("tool_call block has no tool call")
        return {
            "type": "tool_call",
            "name": call.name,
            "arguments": call.arguments,
        }


class ResponseParser:
    def __init__(self, aliases: ToolAliasConfig):
        self._aliases = aliases

    def parse_direct(
        self,
        protocol: str,
        document: dict[str, Any],
    ) -> RecordedResponse | None:
        if protocol == "openai":
            return self._direct_openai(document)
        if protocol == "anthropic":
            return self._direct_anthropic(document)
        return None

    def stream_parser(self, protocol: str) -> StreamParser:
        return StreamParser(protocol, self._aliases)

    def _direct_openai(
        self, document: dict[str, Any]
    ) -> RecordedResponse | None:
        choices = document.get("choices")
        if not isinstance(choices, list) or not choices:
            return None
        choice = choices[0]
        if not isinstance(choice, dict):
            return None
        message = choice.get("message")
        if not isinstance(message, dict):
            return None
        blocks = self._openai_blocks(
            message,
            self._extract_text(document),
            self._extract_reasoning(document),
        )
        if not blocks:
            return None
        has_tool_calls = any(
            block.kind == "tool_call" for block in blocks
        )
        usage = document.get("usage")
        output_tokens = 0
        if isinstance(usage, dict):
            output_tokens = self._non_negative_int(
                usage.get("completion_tokens")
            )
        return RecordedResponse(
            protocol="openai",
            stream=False,
            model=self._string_or_none(document.get("model")),
            blocks=tuple(blocks),
            stop=normalize_stop(
                "openai",
                choice.get("finish_reason"),
                has_tool_calls,
            ),
            output_tokens=output_tokens,
        )

    def _direct_anthropic(
        self, document: dict[str, Any]
    ) -> RecordedResponse | None:
        content = document.get("content")
        if not isinstance(content, list):
            return None
        blocks: list[ResponseBlock] = []
        for item in content:
            if not isinstance(item, dict):
                continue
            item_type = item.get("type")
            if item_type == "text":
                text = item.get("text")
                if isinstance(text, str) and text:
                    blocks.append(
                        ResponseBlock(
                            kind="message",
                            fragments=(text,),
                            tool_call=None,
                        )
                    )
            elif item_type == "thinking":
                thinking = item.get("thinking")
                if isinstance(thinking, str) and thinking:
                    blocks.append(
                        ResponseBlock(
                            kind="reasoning",
                            fragments=(thinking,),
                            tool_call=None,
                        )
                    )
            elif item_type == "tool_use":
                name = item.get("name")
                arguments = item.get("input")
                if isinstance(name, str) and name:
                    if not isinstance(arguments, dict):
                        arguments = {}
                    canonical_name, canonical_arguments = (
                        self._canonicalize(name, arguments)
                    )
                    blocks.append(
                        ResponseBlock(
                            kind="tool_call",
                            fragments=(),
                            tool_call=ToolCall(
                                name=canonical_name,
                                arguments=canonical_arguments,
                            ),
                        )
                    )
        if not blocks:
            return None
        has_tool_calls = any(
            block.kind == "tool_call" for block in blocks
        )
        usage = document.get("usage")
        output_tokens = 0
        if isinstance(usage, dict):
            output_tokens = self._non_negative_int(
                usage.get("output_tokens")
            )
        return RecordedResponse(
            protocol="anthropic",
            stream=False,
            model=self._string_or_none(document.get("model")),
            blocks=tuple(blocks),
            stop=normalize_stop(
                "anthropic",
                document.get("stop_reason"),
                has_tool_calls,
            ),
            output_tokens=output_tokens,
        )

    def _openai_blocks(
        self,
        message: dict[str, Any],
        content_fragments: list[str],
        reasoning_fragments: list[str],
    ) -> list[ResponseBlock]:
        blocks: list[ResponseBlock] = []
        if reasoning_fragments:
            blocks.append(
                ResponseBlock(
                    kind="reasoning",
                    fragments=tuple(reasoning_fragments),
                    tool_call=None,
                )
            )
        if content_fragments:
            blocks.append(
                ResponseBlock(
                    kind="message",
                    fragments=tuple(content_fragments),
                    tool_call=None,
                )
            )
        tool_calls = message.get("tool_calls")
        if isinstance(tool_calls, list):
            for item in tool_calls:
                if not isinstance(item, dict):
                    continue
                function = item.get("function")
                if not isinstance(function, dict):
                    continue
                name = function.get("name")
                if not isinstance(name, str) or not name:
                    continue
                arguments = self._parse_arguments(
                    function.get("arguments")
                )
                canonical_name, canonical_arguments = (
                    self._canonicalize(name, arguments)
                )
                blocks.append(
                    ResponseBlock(
                        kind="tool_call",
                        fragments=(),
                        tool_call=ToolCall(
                            name=canonical_name,
                            arguments=canonical_arguments,
                        ),
                    )
                )
        return blocks

    def _canonicalize(
        self,
        name: str,
        arguments: dict[str, Any],
    ) -> tuple[str, dict[str, Any]]:
        canonical_name = self._aliases.canonical_name(name)
        canonical_arguments = self._aliases.canonical_arguments(
            canonical_name, arguments
        )
        return canonical_name, canonical_arguments

    @staticmethod
    def _extract_text(document: dict[str, Any]) -> list[str]:
        choices = document.get("choices")
        if not isinstance(choices, list) or not choices:
            return []
        choice = choices[0]
        if not isinstance(choice, dict):
            return []
        message = choice.get("message")
        if not isinstance(message, dict):
            return []
        content = message.get("content")
        if isinstance(content, str):
            return [content] if content else []
        if isinstance(content, list):
            fragments: list[str] = []
            for part in content:
                if not isinstance(part, dict):
                    continue
                text = part.get("text")
                if isinstance(text, str) and text:
                    fragments.append(text)
            return fragments
        return []

    @staticmethod
    def _extract_reasoning(document: dict[str, Any]) -> list[str]:
        choices = document.get("choices")
        if not isinstance(choices, list) or not choices:
            return []
        choice = choices[0]
        if not isinstance(choice, dict):
            return []
        message = choice.get("message")
        if not isinstance(message, dict):
            return []
        reasoning = message.get("reasoning_content")
        if isinstance(reasoning, str) and reasoning:
            return [reasoning]
        return []

    @staticmethod
    def _parse_arguments(value: object) -> dict[str, Any]:
        if not isinstance(value, str) or not value:
            return {}
        try:
            parsed = json.loads(value)
        except ValueError:
            return {}
        return parsed if isinstance(parsed, dict) else {}

    @staticmethod
    def _string_or_none(value: object) -> str | None:
        return value if isinstance(value, str) else None

    @staticmethod
    def _non_negative_int(value: object) -> int:
        if isinstance(value, bool) or not isinstance(value, int):
            return 0
        return max(0, value)


class StreamParser:
    def __init__(self, protocol: str, aliases: ToolAliasConfig):
        self._protocol = protocol
        self._aliases = aliases
        self._finished = False
        self._model: str | None = None
        self._stop_reason: str | None = None
        self._output_tokens = 0
        if protocol == "openai":
            self._openai = _OpenAIStreamAccumulator()
            self._anthropic = None
        else:
            self._openai = None
            self._anthropic = _AnthropicStreamAccumulator()

    def feed_line(self, line: bytes) -> None:
        text = line.decode("utf-8", errors="replace").rstrip("\r\n")
        if self._protocol == "openai":
            self._feed_openai(text)
        else:
            self._feed_anthropic(text)

    def result(self) -> RecordedResponse | None:
        if not self._finished:
            return None
        if self._openai is not None:
            blocks = self._openai.blocks(self._aliases)
        else:
            assert self._anthropic is not None
            blocks = self._anthropic.blocks(self._aliases)
        if not blocks:
            return None
        has_tool_calls = any(
            block.kind == "tool_call" for block in blocks
        )
        return RecordedResponse(
            protocol=self._protocol,
            stream=True,
            model=self._model,
            blocks=tuple(blocks),
            stop=normalize_stop(
                self._protocol,
                self._stop_reason,
                has_tool_calls,
            ),
            output_tokens=self._output_tokens,
        )

    def _feed_openai(self, text: str) -> None:
        if not text.startswith("data:"):
            return
        payload = text[5:].strip()
        if payload == "[DONE]":
            self._finished = True
            return
        if not payload.startswith("{"):
            return
        try:
            event = json.loads(payload)
        except ValueError:
            return
        if not isinstance(event, dict):
            return
        if isinstance(event.get("model"), str):
            self._model = event["model"]
        choices = event.get("choices")
        if isinstance(choices, list) and choices:
            choice = choices[0]
            if isinstance(choice, dict):
                if choice.get("finish_reason") is not None:
                    self._stop_reason = choice["finish_reason"]
                delta = choice.get("delta")
                if isinstance(delta, dict):
                    assert self._openai is not None
                    self._openai.feed_delta(delta)
        usage = event.get("usage")
        if isinstance(usage, dict):
            tokens = usage.get("completion_tokens")
            if isinstance(tokens, int) and not isinstance(tokens, bool):
                self._output_tokens = max(0, tokens)

    def _feed_anthropic(self, text: str) -> None:
        assert self._anthropic is not None
        if text.startswith("event:"):
            self._anthropic.current_event = text[6:].strip()
            return
        if not text.startswith("data:"):
            return
        payload = text[5:].strip()
        if not payload.startswith("{"):
            return
        try:
            event = json.loads(payload)
        except ValueError:
            return
        if not isinstance(event, dict):
            return
        event_type = event.get("type")
        if event_type == "message_start":
            message = event.get("message")
            if isinstance(message, dict) and isinstance(
                message.get("model"), str
            ):
                self._model = message["model"]
        elif event_type == "content_block_start":
            index = event.get("index", 0)
            content_block = event.get("content_block")
            if isinstance(content_block, dict):
                block_type = content_block.get("type")
                if isinstance(block_type, str):
                    self._anthropic.start_block(index, block_type, content_block)
        elif event_type == "content_block_delta":
            index = event.get("index", 0)
            delta = event.get("delta")
            if isinstance(delta, dict):
                self._anthropic.feed_delta(index, delta)
        elif event_type == "message_delta":
            delta = event.get("delta")
            if isinstance(delta, dict) and delta.get("stop_reason") is not None:
                self._stop_reason = delta["stop_reason"]
            usage = event.get("usage")
            if isinstance(usage, dict):
                tokens = usage.get("output_tokens")
                if isinstance(tokens, int) and not isinstance(tokens, bool):
                    self._output_tokens = max(0, tokens)
        elif event_type == "message_stop":
            self._finished = True


class _OpenAIStreamAccumulator:
    def __init__(self) -> None:
        self._content: list[str] = []
        self._reasoning: list[str] = []
        self._tool_calls: dict[int, dict[str, object]] = {}
        self._tool_order: list[int] = []

    def feed_delta(self, delta: dict[str, Any]) -> None:
        content = delta.get("content")
        if isinstance(content, str) and content:
            self._content.append(content)
        reasoning = delta.get("reasoning_content")
        if isinstance(reasoning, str) and reasoning:
            self._reasoning.append(reasoning)
        tool_calls = delta.get("tool_calls")
        if not isinstance(tool_calls, list):
            return
        for item in tool_calls:
            if not isinstance(item, dict):
                continue
            index = item.get("index", 0)
            if isinstance(index, bool) or not isinstance(index, int):
                index = 0
            entry = self._tool_calls.setdefault(
                index,
                {"name": None, "arguments": []},
            )
            if index not in self._tool_order:
                self._tool_order.append(index)
            function = item.get("function")
            if not isinstance(function, dict):
                continue
            name = function.get("name")
            if isinstance(name, str) and name:
                entry["name"] = name
            arguments = function.get("arguments")
            if isinstance(arguments, str) and arguments:
                entry["arguments"].append(arguments)

    def blocks(
        self, aliases: ToolAliasConfig
    ) -> list[ResponseBlock]:
        blocks: list[ResponseBlock] = []
        if self._reasoning:
            blocks.append(
                ResponseBlock(
                    kind="reasoning",
                    fragments=tuple(self._reasoning),
                    tool_call=None,
                )
            )
        if self._content:
            blocks.append(
                ResponseBlock(
                    kind="message",
                    fragments=tuple(self._content),
                    tool_call=None,
                )
            )
        for index in self._tool_order:
            entry = self._tool_calls[index]
            name = entry["name"]
            if not isinstance(name, str) or not name:
                continue
            arguments = self._parse_arguments(
                "".join(entry["arguments"])
            )
            canonical_name = aliases.canonical_name(name)
            canonical_arguments = aliases.canonical_arguments(
                canonical_name, arguments
            )
            blocks.append(
                ResponseBlock(
                    kind="tool_call",
                    fragments=(),
                    tool_call=ToolCall(
                        name=canonical_name,
                        arguments=canonical_arguments,
                    ),
                )
            )
        return blocks

    @staticmethod
    def _parse_arguments(raw: str) -> dict[str, Any]:
        if not raw:
            return {}
        try:
            parsed = json.loads(raw)
        except ValueError:
            return {}
        return parsed if isinstance(parsed, dict) else {}


class _AnthropicStreamAccumulator:
    def __init__(self) -> None:
        self.current_event: str = ""
        self._blocks: dict[int, dict[str, object]] = {}
        self._order: list[int] = []

    def start_block(
        self,
        index: object,
        block_type: str,
        content_block: dict[str, Any],
    ) -> None:
        normalized_index = self._normalize_index(index)
        entry: dict[str, object] = {
            "type": block_type,
            "fragments": [],
            "name": None,
            "arguments": [],
        }
        if block_type == "tool_use":
            name = content_block.get("name")
            if isinstance(name, str) and name:
                entry["name"] = name
        self._blocks[normalized_index] = entry
        if normalized_index not in self._order:
            self._order.append(normalized_index)

    def feed_delta(
        self, index: object, delta: dict[str, Any]
    ) -> None:
        entry = self._blocks.get(self._normalize_index(index))
        if entry is None:
            return
        delta_type = delta.get("type")
        if delta_type == "text_delta":
            text = delta.get("text")
            if isinstance(text, str) and text:
                entry["fragments"].append(text)
        elif delta_type == "thinking_delta":
            thinking = delta.get("thinking")
            if isinstance(thinking, str) and thinking:
                entry["fragments"].append(thinking)
        elif delta_type == "input_json_delta":
            partial = delta.get("partial_json")
            if isinstance(partial, str) and partial:
                entry["arguments"].append(partial)

    def blocks(
        self, aliases: ToolAliasConfig
    ) -> list[ResponseBlock]:
        blocks: list[ResponseBlock] = []
        for index in self._order:
            entry = self._blocks[index]
            block_type = entry["type"]
            fragments = tuple(entry["fragments"])
            if block_type == "text":
                if fragments:
                    blocks.append(
                        ResponseBlock(
                            kind="message",
                            fragments=fragments,
                            tool_call=None,
                        )
                    )
            elif block_type == "thinking":
                if fragments:
                    blocks.append(
                        ResponseBlock(
                            kind="reasoning",
                            fragments=fragments,
                            tool_call=None,
                        )
                    )
            elif block_type == "tool_use":
                name = entry["name"]
                if not isinstance(name, str) or not name:
                    continue
                arguments = self._parse_arguments(
                    "".join(entry["arguments"])
                )
                canonical_name = aliases.canonical_name(name)
                canonical_arguments = aliases.canonical_arguments(
                    canonical_name, arguments
                )
                blocks.append(
                    ResponseBlock(
                        kind="tool_call",
                        fragments=(),
                        tool_call=ToolCall(
                            name=canonical_name,
                            arguments=canonical_arguments,
                        ),
                    )
                )
        return blocks

    @staticmethod
    def _normalize_index(index: object) -> int:
        if isinstance(index, bool) or not isinstance(index, int):
            return 0
        return index

    @staticmethod
    def _parse_arguments(raw: str) -> dict[str, Any]:
        if not raw:
            return {}
        try:
            parsed = json.loads(raw)
        except ValueError:
            return {}
        return parsed if isinstance(parsed, dict) else {}
