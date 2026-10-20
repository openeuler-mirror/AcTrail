from __future__ import annotations

from collections import Counter

from tests.v2.regression.http_llm_sources.assertion import HttpLlmSourcesAssertion


class WebSocketLlmSourcesAssertion(HttpLlmSourcesAssertion):
    """Validate native message provenance from an actual Responses WebSocket session."""

    def require_llm(self, marker: str, minimum_successes: int, **_options) -> dict:
        requests = self.of_kind("llm.request")
        responses = self.of_kind("llm.response")
        calls = self.of_kind("llm.call")
        if not requests or len(calls) != len(requests):
            raise AssertionError("expected one native WebSocket call per request")
        request_owners, response_owners = Counter(), Counter()
        message_ids = set()
        for call in calls:
            request = self.one(call["action_id"], "llm.call.request", "llm.request")
            response = self.one(call["action_id"], "llm.call.response", "llm.response")
            request_owners[request["action_id"]] += 1
            response_owners[response["action_id"]] += 1
            for name, action in (("request", request), ("response", response)):
                if call.get("attributes", {}).get(f"llm.call.{name}_action_id") != action["action_id"]:
                    raise AssertionError("call identity disagrees with its exact link")
                self.require_complete(action)
                sources = self.links[(action["action_id"], f"llm.{name}.websocket_message")]
                if not sources or (name == "request" and len(sources) != 1):
                    raise AssertionError(f"missing/ambiguous native WebSocket {name} sources")
                if self.links.get((action["action_id"], f"llm.{name}.http_message")):
                    raise AssertionError("WebSocket LLM action has a fabricated HTTP source")
                for source_id in sources:
                    source = self.actions.get(source_id)
                    if source is None or source.get("kind") != "websocket.message":
                        raise AssertionError("missing native WebSocket source")
                    attrs = source.get("attributes", {})
                    if attrs.get("websocket.message_id") != source_id:
                        raise AssertionError("WebSocket source does not own its message identity")
                    if name == "request" and action.get("attributes", {}).get("websocket.message_id") != source_id:
                        raise AssertionError("request lost its original WebSocket message identity")
                    self.require_connection(request, response, source)
                    self.require_complete(source)
                    message_ids.add(source_id)
        self.require_single_owners(requests, request_owners, "request")
        self.require_single_owners(responses, response_owners, "response")
        successes = [row for row in responses if row.get("status") == "success"]
        if len(successes) < minimum_successes:
            raise AssertionError("real WebSocket tool turn did not complete enough responses")
        for response in successes:
            attributes = response.get("attributes", {})
            if attributes.get("llm.response.done") != "true":
                raise AssertionError("stored WebSocket response retained its initial attributes")
        if not any(marker in row.get("attributes", {}).get("llm.response.content_text", "") for row in successes):
            raise AssertionError("captured WebSocket response lacks the real answer marker")
        if not self.of_kind("llm.tool_call"):
            raise AssertionError("real WebSocket tool call was not captured")
        return dict(calls=len(calls), requests=len(requests), responses=len(responses),
                    successful_rounds=len(successes), source_messages=len(message_ids),
                    request_ids=[row["action_id"] for row in requests])

    def require_http_only(self) -> dict:
        if any(self.of_kind(kind) for kind in ("llm.call", "llm.request", "llm.response")):
            raise AssertionError("LLM-disabled run emitted LLM actions")
        messages = self.of_kind("websocket.message")
        if not messages:
            raise AssertionError("native WebSocket observations disappeared with LLM disabled")
        for message in messages:
            self.require_complete(message)
        return {"websocket_messages": len(messages)}

    def require_connection(self, *actions: dict) -> None:
        identities = set()
        for action in actions:
            attrs = action.get("attributes", {})
            connection = attrs.get("websocket.connection_id")
            process = action.get("process", {}).get("process_id")
            if not connection or process is None or action.get("trace_id_raw") != self.trace_id:
                raise AssertionError("native WebSocket action lacks authoritative connection identity")
            identities.add((connection, process))
        if len(identities) != 1:
            raise AssertionError("WebSocket source link crosses process or connection")
