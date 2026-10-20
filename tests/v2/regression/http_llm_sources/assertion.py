from __future__ import annotations

from collections import Counter, defaultdict


class HttpLlmSourcesAssertion:
    """Validate persisted HTTP source evidence from real agent executions."""

    def __init__(self, document: dict, trace_id: int, closed_at: int, protocol: str = "http1"):
        self.trace_id = trace_id
        self.closed_at = closed_at
        self.protocol = protocol
        rows = document.get("actions", [])
        self.actions = {row["action_id"]: row for row in rows}
        if len(rows) != len(self.actions):
            raise AssertionError("duplicate action identities in viewer output")
        self.links = defaultdict(set)
        for link in document.get("links", []):
            if link.get("valid", False):
                self.links[(link["parent_action_id"], link["role"])].add(
                    link["child_action_id"]
                )

    def require_llm(self, marker: str, minimum_successes: int, *, require_connection_reuse: bool = True) -> dict:
        requests = self.of_kind("llm.request")
        responses = self.of_kind("llm.response")
        calls = self.of_kind("llm.call")
        if not requests or len(calls) != len(requests):
            raise AssertionError(
                f"expected one call for every LLM request: requests={len(requests)}, "
                f"responses={len(responses)}, calls={len(calls)}"
            )
        request_sources = self.sources(requests, "llm.request.http_message")
        response_sources = self.sources(responses, "llm.response.http_message")
        request_owners = Counter()
        response_owners = Counter()
        pairs = []
        for call in calls:
            request = self.one(call["action_id"], "llm.call.request", "llm.request")
            response = self.one(call["action_id"], "llm.call.response", "llm.response")
            request_owners[request["action_id"]] += 1
            response_owners[response["action_id"]] += 1
            attrs = call.get("attributes", {})
            if attrs.get("llm.call.request_action_id") != request["action_id"]:
                raise AssertionError("call request attribute disagrees with its link")
            if attrs.get("llm.call.response_action_id") != response["action_id"]:
                raise AssertionError("call response attribute disagrees with its link")
            http_request = request_sources[request["action_id"]]
            http_response = response_sources[response["action_id"]]
            linked_http = self.one(
                http_request["action_id"], "http.request.http_response", "http.message"
            )
            if linked_http["action_id"] != http_response["action_id"]:
                raise AssertionError("HTTP exchange and LLM call disagree on response ownership")
            if http_response.get("attributes", {}).get("http.request.action_id") != http_request["action_id"]:
                raise AssertionError("HTTP response request attribute disagrees with source link")
            self.require_context(request, response, http_request, http_response)
            for row in (request, response):
                self.require_complete(row)
            pairs.append((request, response))
        self.require_single_owners(requests, request_owners, "request")
        self.require_single_owners(responses, response_owners, "response")
        reverse_responses = {row["action_id"]: key for key, row in response_sources.items()}
        http_errors = 0
        http_404s = 0
        for request in request_sources.values():
            response = self.one(request["action_id"], "http.request.http_response", "http.message")
            if self.http_status(response) < 400:
                continue
            http_errors += 1
            http_404s += int(self.http_status(response) == 404)
            llm_id = reverse_responses.get(response["action_id"])
            if llm_id is None:
                raise AssertionError(f"failed HTTP response has no LLM response: {response['action_id']}")
            llm_response = self.actions[llm_id]
            if llm_response.get("status") != "error":
                raise AssertionError("failed HTTP exchange did not produce an error LLM response")
            if self.http_status(llm_response) != self.http_status(response):
                raise AssertionError("LLM error HTTP status differs from its HTTP source")
        successful = [response for _, response in pairs if response.get("status") == "success"]
        if len(successful) < minimum_successes:
            raise AssertionError(f"expected at least {minimum_successes} successful rounds, observed {len(successful)}")
        if not any(marker in response.get("attributes", {}).get("llm.response.content_text", "") for response in successful):
            raise AssertionError("captured successful response has no real answer marker")
        if not self.of_kind("llm.tool_call"):
            raise AssertionError("real file-reading tool turn was not observed")
        connections = defaultdict(list)
        for request in request_sources.values():
            attrs = request.get("attributes", {})
            key = (request["process"]["process_id"], attrs.get("payload.stream_key") or attrs.get("stream_key"))
            connections[key].append(request)
        reused = [rows for rows in connections.values() if len(rows) >= 2]
        if require_connection_reuse and not reused:
            raise AssertionError("same-connection coverage missing: no connection carried multiple LLM requests")
        retry_success_connections = 0
        for rows in reused:
            statuses = [self.http_status(self.one(row["action_id"], "http.request.http_response", "http.message")) for row in rows]
            retry_success_connections += int(any(status >= 400 for status in statuses) and any(200 <= status < 300 for status in statuses))
        return {
            "reused_connections": len(reused),
            "retry_success_connections": retry_success_connections,
            "calls": len(calls), "requests": len(requests), "responses": len(responses),
            "successful_rounds": len(successful), "http_errors": http_errors, "http_404s": http_404s,
            "request_ids": [row["action_id"] for row in requests],
        }

    def require_http_only(self) -> dict:
        if any(self.of_kind(kind) for kind in ("llm.call", "llm.request", "llm.response")):
            raise AssertionError("HTTP-only execution unexpectedly emitted LLM actions")
        exchanges = []
        for (parent, role), children in self.links.items():
            if role != "http.request.http_response":
                continue
            request = self.actions[parent]
            self.require_protocol(request)
            for child in children:
                response = self.actions[child]
                self.require_protocol(response)
                self.require_context(request, response)
                if response.get("attributes", {}).get("http.request.action_id") != parent:
                    raise AssertionError("HTTP-only source identity differs from exchange link")
                exchanges.append((parent, child))
        if not exchanges:
            raise AssertionError("ordinary HTTP facts disappeared when LLM was absent/disabled")
        return {"http_exchanges": len(exchanges)}

    def sources(self, actions: list[dict], role: str) -> dict:
        result = {}
        for action in actions:
            source = self.one(action["action_id"], role, "http.message")
            self.require_protocol(source)
            message_id = action.get("attributes", {}).get("http.message_id")
            if message_id != source["action_id"] or source.get("attributes", {}).get("http.message_id") != message_id:
                raise AssertionError("LLM source identity differs from linked HTTP message identity")
            result[action["action_id"]] = source
        counts = Counter(row["action_id"] for row in result.values())
        if any(count != 1 for count in counts.values()):
            raise AssertionError(f"multiple LLM projections share one source message: {role}")
        return result

    def one(self, parent: str, role: str, kind: str) -> dict:
        children = self.links.get((parent, role), set())
        if len(children) != 1:
            raise AssertionError(f"{parent}: expected one {role} link, found {len(children)}")
        action = self.actions.get(next(iter(children)))
        if not action or action.get("kind") != kind:
            raise AssertionError(f"{parent}: missing/wrong-kind target for {role}")
        return action

    def of_kind(self, kind: str) -> list[dict]:
        return [row for row in self.actions.values() if row.get("kind") == kind]

    def require_complete(self, action: dict) -> None:
        if action.get("completeness") != "complete" or action.get("end_time_unix_nanos") is None:
            raise AssertionError(f"normal workload has unfinished action {action['action_id']}")
        if action.get("attributes", {}).get("actrail.action.finalized_on_trace_close") == "true":
            raise AssertionError(f"normal workload was finalized on trace close: {action['action_id']}")
        if int(action["end_time_unix_nanos"]) >= self.closed_at:
            raise AssertionError(f"normal action did not finish before trace close: {action['action_id']}")

    def require_context(self, *actions: dict) -> None:
        identities = []
        http2_streams = []
        for action in actions:
            attrs = action.get("attributes", {})
            process = action.get("process", {}).get("process_id")
            stream = attrs.get("payload.stream_key") or attrs.get("stream_key")
            if action.get("trace_id_raw") != self.trace_id or process is None or not stream:
                raise AssertionError(f"missing/wrong trace, process or stream identity: {action['action_id']}")
            identities.append((process, stream))
            if self.protocol == "http2":
                stream_id = attrs.get("http.request.stream_id") or attrs.get("http.response.stream_id") or attrs.get("stream_id")
                if stream_id is None or int(stream_id) < 1:
                    raise AssertionError(f"missing HTTP/2 stream identity: {action['action_id']}")
                http2_streams.append(int(stream_id))
        if len(set(identities)) != 1:
            raise AssertionError("HTTP/LLM source link crosses process or connection identity")
        if http2_streams and len(set(http2_streams)) != 1:
            raise AssertionError("HTTP/LLM source link crosses HTTP/2 streams")

    def require_protocol(self, action: dict) -> None:
        attrs = action.get("attributes", {})
        protocol = attrs.get("network.protocol.version") or attrs.get("http.request.protocol") or attrs.get("http.response.protocol")
        allowed = {
            "http1": {"http/1.0", "http/1.1", "http1", "http/1"},
            "http2": {"http/2", "http/2.0", "http2", "h2"},
        }[self.protocol]
        if str(protocol).lower() not in allowed:
            raise AssertionError(f"{self.protocol} coverage required, observed protocol={protocol!r}")

    @staticmethod
    def http_status(action: dict) -> int:
        attrs = action.get("attributes", {})
        return int(attrs.get("http.response.status_code", attrs.get("status_code", "0")))

    @staticmethod
    def require_single_owners(actions: list[dict], counts: Counter, label: str) -> None:
        if set(counts) != {row["action_id"] for row in actions} or any(count != 1 for count in counts.values()):
            raise AssertionError(f"LLM {label} has missing or duplicate call owner")
