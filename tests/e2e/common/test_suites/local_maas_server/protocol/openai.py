from __future__ import annotations

import hashlib
import json
from typing import Any, Iterator

from scenario.model import (
    ResponseBlock,
    ScenarioEmission,
    ScenarioRequest,
    ToolDefinition,
    UsageSnapshot,
)

from .interface import (
    ProtocolAdapter,
    ProtocolFrame,
    ProtocolRequestError,
    ProtocolResponse,
)


class OpenAIChatAdapter(ProtocolAdapter):
    @property
    def name(self) -> str:
        return "openai"

    @property
    def service_name(self) -> str:
        return "OpenAI-compatible"

    @property
    def paths(self) -> frozenset[str]:
        return frozenset({"/chat/completions", "/v1/chat/completions"})

    @property
    def canonical_path(self) -> str:
        return "/v1/chat/completions"

    @property
    def model_paths(self) -> frozenset[str]:
        return frozenset({"/models", "/v1/models"})

    def decode_request(self, document: dict[str, Any]) -> ScenarioRequest:
        model = self._required_string(document.get("model"), "model")
        if not isinstance(document.get("messages"), list):
            raise ProtocolRequestError(
                "invalid_messages", "messages must be an array"
            )
        stream = document.get("stream", False)
        if not isinstance(stream, bool):
            raise ProtocolRequestError(
                "invalid_stream", "stream must be a boolean"
            )
        include_usage = False
        stream_options = document.get("stream_options")
        if stream_options is not None:
            if not stream:
                raise ProtocolRequestError(
                    "invalid_stream_options",
                    "stream_options is only valid when stream is true",
                )
            if not isinstance(stream_options, dict):
                raise ProtocolRequestError(
                    "invalid_stream_options",
                    "stream_options must be an object",
                )
            include_usage = stream_options.get("include_usage", False)
            if not isinstance(include_usage, bool):
                raise ProtocolRequestError(
                    "invalid_stream_options",
                    "stream_options.include_usage must be a boolean",
                )
        return ScenarioRequest(
            protocol=self.name,
            stream=stream,
            model=model,
            include_usage=include_usage,
            input_tokens=self.input_tokens(document),
            tools=self._decode_tools(document.get("tools", [])),
        )

    def encode_response(
        self,
        request: ScenarioRequest,
        emission: ScenarioEmission,
        default_model: str,
    ) -> ProtocolResponse:
        if request.stream:
            return ProtocolResponse(
                status=200,
                media_type="text/event-stream; charset=utf-8",
                frames=self._stream_frames(emission, request, default_model),
            )
        payload = self._direct_payload(emission, request, default_model)
        return ProtocolResponse(
            status=200,
            media_type="application/json; charset=utf-8",
            body=self._json_bytes(payload),
        )

    def encode_error(
        self, status: int, code: str, message: str
    ) -> ProtocolResponse:
        payload = {
            "error": {
                "message": message,
                "type": (
                    "invalid_request_error" if status < 500 else "server_error"
                ),
                "param": None,
                "code": code,
            }
        }
        return ProtocolResponse(
            status=status,
            media_type="application/json; charset=utf-8",
            body=self._json_bytes(payload),
        )

    def _direct_payload(
        self,
        emission: ScenarioEmission,
        request: ScenarioRequest,
        default_model: str,
    ) -> dict[str, Any]:
        response = emission.template.response
        message: dict[str, Any] = {"role": "assistant"}
        content = "".join(
            block.text for block in response.blocks if block.kind == "message"
        )
        reasoning = "".join(
            block.text
            for block in response.blocks
            if block.kind == "reasoning"
        )
        message["content"] = content or None
        if reasoning:
            message["reasoning_content"] = reasoning
        tool_calls = [
            self._tool_call(block, emission, index)
            for index, block in enumerate(response.blocks)
            if block.kind == "tool_call"
        ]
        if tool_calls:
            message["tool_calls"] = tool_calls
        return {
            "id": self._response_id(emission),
            "object": "chat.completion",
            "created": 0,
            "model": response.model or request.model or default_model,
            "choices": [
                {
                    "index": 0,
                    "message": message,
                    "finish_reason": self._finish_reason(response.stop),
                }
            ],
            "usage": self._usage(emission.usage),
        }

    def _stream_frames(
        self,
        emission: ScenarioEmission,
        request: ScenarioRequest,
        default_model: str,
    ) -> Iterator[ProtocolFrame]:
        response = emission.template.response
        model = response.model or request.model or default_model
        response_id = self._response_id(emission)
        yield self._data_frame(
            self._chunk(
                response_id,
                model,
                {"role": "assistant"},
                None,
                request.include_usage,
            )
        )
        tool_index = 0
        for block_index, block in enumerate(response.blocks):
            if block.kind == "reasoning":
                for fragment in block.fragments:
                    yield self._data_frame(
                        self._chunk(
                            response_id,
                            model,
                            {"reasoning_content": fragment},
                            None,
                            request.include_usage,
                        )
                    )
            elif block.kind == "message":
                for fragment in block.fragments:
                    yield self._data_frame(
                        self._chunk(
                            response_id,
                            model,
                            {"content": fragment},
                            None,
                            request.include_usage,
                        )
                    )
            else:
                tool_call = self._required_tool_call(block)
                yield self._data_frame(
                    self._chunk(
                        response_id,
                        model,
                        {
                            "tool_calls": [
                                {
                                    "index": tool_index,
                                    "id": self._wire_tool_call_id(
                                        emission, block_index
                                    ),
                                    "type": "function",
                                    "function": {
                                        "name": tool_call.name,
                                        "arguments": "",
                                    },
                                }
                            ]
                        },
                        None,
                        request.include_usage,
                    )
                )
                arguments = self._arguments(tool_call.arguments)
                yield self._data_frame(
                    self._chunk(
                        response_id,
                        model,
                        {
                            "tool_calls": [
                                {
                                    "index": tool_index,
                                    "function": {"arguments": arguments},
                                }
                            ]
                        },
                        None,
                        request.include_usage,
                    )
                )
                tool_index += 1
        yield self._data_frame(
            self._chunk(
                response_id,
                model,
                {},
                self._finish_reason(response.stop),
                request.include_usage,
            )
        )
        if request.include_usage:
            yield self._data_frame(
                {
                    "id": response_id,
                    "object": "chat.completion.chunk",
                    "created": 0,
                    "model": model,
                    "choices": [],
                    "usage": self._usage(emission.usage),
                }
            )
        yield ProtocolFrame(payload=b"data: [DONE]\n\n")

    def _tool_call(
        self, block: ResponseBlock, emission: ScenarioEmission, block_index: int
    ) -> dict[str, Any]:
        tool_call = self._required_tool_call(block)
        return {
            "id": self._wire_tool_call_id(emission, block_index),
            "type": "function",
            "function": {
                "name": tool_call.name,
                "arguments": self._arguments(tool_call.arguments),
            },
        }

    @staticmethod
    def _required_tool_call(block: ResponseBlock):
        if block.tool_call is None:
            raise RuntimeError("validated tool_call block is missing tool data")
        return block.tool_call

    def _decode_tools(self, value: object) -> tuple[ToolDefinition, ...]:
        if not isinstance(value, list):
            raise ProtocolRequestError(
                "invalid_tools", "tools must be an array"
            )
        tools = []
        for index, raw_tool in enumerate(value):
            if not isinstance(raw_tool, dict):
                raise ProtocolRequestError(
                    "invalid_tools", f"tools[{index}] must be an object"
                )
            if raw_tool.get("type") != "function":
                raise ProtocolRequestError(
                    "invalid_tools",
                    f"tools[{index}].type must be function",
                )
            function = raw_tool.get("function")
            if not isinstance(function, dict):
                raise ProtocolRequestError(
                    "invalid_tools",
                    f"tools[{index}].function must be an object",
                )
            name = self._required_string(
                function.get("name"),
                f"tools[{index}].function.name",
            )
            parameters = function.get("parameters", {})
            if not isinstance(parameters, dict):
                raise ProtocolRequestError(
                    "invalid_tools",
                    f"tools[{index}].function.parameters must be an object",
                )
            tools.append(
                ToolDefinition(name=name, input_schema=parameters)
            )
        return tuple(tools)

    @staticmethod
    def _required_string(value: object, field: str) -> str:
        if not isinstance(value, str) or not value:
            raise ProtocolRequestError(
                f"invalid_{field}", f"{field} must be a non-empty string"
            )
        return value

    @staticmethod
    def _arguments(arguments: dict[str, Any]) -> str:
        return json.dumps(arguments, ensure_ascii=False, separators=(",", ":"))

    @staticmethod
    def _finish_reason(stop: str) -> str:
        return {
            "complete": "stop",
            "tool_call": "tool_calls",
            "length": "length",
        }[stop]

    @staticmethod
    def _usage(usage: UsageSnapshot) -> dict[str, int]:
        return {
            "prompt_tokens": usage.input_tokens,
            "completion_tokens": usage.output_tokens,
            "total_tokens": usage.input_tokens + usage.output_tokens,
        }

    @staticmethod
    def _chunk(
        response_id: str,
        model: str,
        delta: dict[str, Any],
        finish_reason: str | None,
        include_usage: bool,
    ) -> dict[str, Any]:
        chunk = {
            "id": response_id,
            "object": "chat.completion.chunk",
            "created": 0,
            "model": model,
            "choices": [
                {
                    "index": 0,
                    "delta": delta,
                    "finish_reason": finish_reason,
                }
            ],
        }
        if include_usage:
            chunk["usage"] = None
        return chunk

    @classmethod
    def _data_frame(cls, payload: dict[str, Any]) -> ProtocolFrame:
        return ProtocolFrame(
            payload=b"data: " + cls._json_bytes(payload) + b"\n\n"
        )

    @staticmethod
    def _json_bytes(payload: dict[str, Any]) -> bytes:
        return json.dumps(
            payload, ensure_ascii=False, separators=(",", ":")
        ).encode("utf-8")

    @staticmethod
    def _wire_tool_call_id(
        emission: ScenarioEmission, block_index: int
    ) -> str:
        return f"call_local_{emission.index}_{block_index}"

    @staticmethod
    def _response_id(emission: ScenarioEmission) -> str:
        value = str(emission.index).encode("ascii")
        digest = hashlib.sha256(value).hexdigest()[:16]
        return f"chatcmpl-local-{digest}"
