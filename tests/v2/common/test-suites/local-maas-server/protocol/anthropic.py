from __future__ import annotations

import hashlib
import json
from typing import Any, Iterator

from scenario.model import (
    ResponseBlock,
    ScenarioEmission,
    ScenarioRequest,
    UsageSnapshot,
)

from .interface import (
    ProtocolAdapter,
    ProtocolFrame,
    ProtocolRequestError,
    ProtocolResponse,
)


class AnthropicMessagesAdapter(ProtocolAdapter):
    @property
    def name(self) -> str:
        return "anthropic"

    @property
    def service_name(self) -> str:
        return "Anthropic"

    @property
    def paths(self) -> frozenset[str]:
        return frozenset({"/messages", "/v1/messages"})

    @property
    def canonical_path(self) -> str:
        return "/v1/messages"

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
        max_tokens = document.get("max_tokens")
        if (
            isinstance(max_tokens, bool)
            or not isinstance(max_tokens, int)
            or max_tokens <= 0
        ):
            raise ProtocolRequestError(
                "invalid_max_tokens",
                "Anthropic requests require a positive integer max_tokens",
            )
        return ScenarioRequest(
            protocol=self.name,
            stream=stream,
            model=model,
            include_usage=True,
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
        error_type = {
            400: "invalid_request_error",
            401: "authentication_error",
            403: "permission_error",
            404: "not_found_error",
            409: "conflict_error",
            413: "request_too_large",
            429: "rate_limit_error",
        }.get(status, "api_error" if status >= 500 else "invalid_request_error")
        request_digest = hashlib.sha256(
            f"{status}\0{code}\0{message}".encode("utf-8")
        ).hexdigest()[:16]
        payload = {
            "type": "error",
            "error": {
                "type": error_type,
                "message": f"{message} ({code})",
            },
            "request_id": f"req_local_{request_digest}",
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
        return {
            "id": self._response_id(emission),
            "type": "message",
            "role": "assistant",
            "model": response.model or request.model or default_model,
            "content": [
                self._content_block(block, emission, index)
                for index, block in enumerate(response.blocks)
            ],
            "stop_reason": self._stop_reason(response.stop),
            "stop_sequence": None,
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
        yield self._event_frame(
            "message_start",
            {
                "type": "message_start",
                "message": {
                    "id": self._response_id(emission),
                    "type": "message",
                    "role": "assistant",
                    "model": model,
                    "content": [],
                    "stop_reason": None,
                    "stop_sequence": None,
                    "usage": {
                        "input_tokens": emission.usage.input_tokens,
                        "output_tokens": 0,
                    },
                },
            }
        )
        for index, block in enumerate(response.blocks):
            yield from self._stream_block(block, emission, index)
        yield self._event_frame(
            "message_delta",
            {
                "type": "message_delta",
                "delta": {
                    "stop_reason": self._stop_reason(response.stop),
                    "stop_sequence": None,
                },
                "usage": {"output_tokens": emission.usage.output_tokens},
            }
        )
        yield self._event_frame(
            "message_stop", {"type": "message_stop"}
        )

    def _stream_block(
        self,
        block: ResponseBlock,
        emission: ScenarioEmission,
        index: int,
    ) -> Iterator[ProtocolFrame]:
        if block.kind == "reasoning":
            yield self._event_frame(
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": index,
                    "content_block": {
                        "type": "thinking",
                        "thinking": "",
                        "signature": "",
                    },
                }
            )
            for fragment in block.fragments:
                yield self._event_frame(
                    "content_block_delta",
                    {
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "thinking_delta",
                            "thinking": fragment,
                        },
                    }
                )
            signature = self._signature(emission, index)
            yield self._event_frame(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {
                        "type": "signature_delta",
                        "signature": signature,
                    },
                }
            )
        elif block.kind == "message":
            yield self._event_frame(
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": index,
                    "content_block": {"type": "text", "text": ""},
                }
            )
            for fragment in block.fragments:
                yield self._event_frame(
                    "content_block_delta",
                    {
                        "type": "content_block_delta",
                        "index": index,
                        "delta": {
                            "type": "text_delta",
                            "text": fragment,
                        },
                    }
                )
        else:
            tool_call = self._required_tool_call(block)
            yield self._event_frame(
                "content_block_start",
                {
                    "type": "content_block_start",
                    "index": index,
                    "content_block": {
                        "type": "tool_use",
                        "id": self._wire_tool_call_id(emission, index),
                        "name": tool_call.name,
                        "input": {},
                    },
                }
            )
            partial_json = json.dumps(
                tool_call.arguments,
                ensure_ascii=False,
                separators=(",", ":"),
            )
            yield self._event_frame(
                "content_block_delta",
                {
                    "type": "content_block_delta",
                    "index": index,
                    "delta": {
                        "type": "input_json_delta",
                        "partial_json": partial_json,
                    },
                }
            )
        yield self._event_frame(
            "content_block_stop",
            {"type": "content_block_stop", "index": index},
        )

    def _content_block(
        self,
        block: ResponseBlock,
        emission: ScenarioEmission,
        index: int,
    ) -> dict[str, Any]:
        if block.kind == "reasoning":
            return {
                "type": "thinking",
                "thinking": block.text,
                "signature": self._signature(emission, index),
            }
        if block.kind == "message":
            return {"type": "text", "text": block.text}
        tool_call = self._required_tool_call(block)
        return {
            "type": "tool_use",
            "id": self._wire_tool_call_id(emission, index),
            "name": tool_call.name,
            "input": tool_call.arguments,
        }

    @staticmethod
    def _required_tool_call(block: ResponseBlock):
        if block.tool_call is None:
            raise RuntimeError("validated tool_call block is missing tool data")
        return block.tool_call

    @staticmethod
    def _required_string(value: object, field: str) -> str:
        if not isinstance(value, str) or not value:
            raise ProtocolRequestError(
                f"invalid_{field}", f"{field} must be a non-empty string"
            )
        return value

    @staticmethod
    def _stop_reason(stop: str) -> str:
        return {
            "complete": "end_turn",
            "tool_call": "tool_use",
            "length": "max_tokens",
        }[stop]

    @staticmethod
    def _usage(usage: UsageSnapshot) -> dict[str, int]:
        return {
            "input_tokens": usage.input_tokens,
            "output_tokens": usage.output_tokens,
        }

    @classmethod
    def _event_frame(
        cls, event_type: str, payload: dict[str, Any]
    ) -> ProtocolFrame:
        return ProtocolFrame(
            payload=(
                f"event: {event_type}\ndata: ".encode("utf-8")
                + cls._json_bytes(payload)
                + b"\n\n"
            )
        )

    @staticmethod
    def _json_bytes(payload: dict[str, Any]) -> bytes:
        return json.dumps(
            payload, ensure_ascii=False, separators=(",", ":")
        ).encode("utf-8")

    @staticmethod
    def _signature(emission: ScenarioEmission, index: int) -> str:
        raw = f"{emission.index}\0{index}".encode("utf-8")
        return f"local-maas-{hashlib.sha256(raw).hexdigest()[:16]}"

    @staticmethod
    def _wire_tool_call_id(
        emission: ScenarioEmission, block_index: int
    ) -> str:
        return f"toolu_local_{emission.index}_{block_index}"

    @staticmethod
    def _response_id(emission: ScenarioEmission) -> str:
        value = str(emission.index).encode("ascii")
        digest = hashlib.sha256(value).hexdigest()[:16]
        return f"msg_local_{digest}"
