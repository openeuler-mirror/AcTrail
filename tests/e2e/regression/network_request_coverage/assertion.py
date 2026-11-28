"""Check recorded failed LLM and ordinary HTTP exchanges."""
from collections import Counter, defaultdict


class NetworkRequestCoverageAssertion:
    def __init__(self, document: dict, trace_id: int):
        rows = document.get("actions", [])
        self.actions = {row["action_id"]: row for row in rows}
        if len(rows) != len(self.actions):
            raise AssertionError("duplicate action identities")
        self.trace_id = trace_id
        self.links = defaultdict(set)
        for link in document.get("links", []):
            if link.get("valid", False):
                self.links[(link["parent_action_id"], link["role"])].add(link["child_action_id"])

    def require_failed_llm(self) -> dict:
        requests = self.of_kind("llm.request")
        responses = self.of_kind("llm.response")
        calls = self.of_kind("llm.call")
        if not requests or len(calls) != len(requests):
            raise AssertionError("failed model task lacks recorded LLM requests or calls")
        request_owners, response_owners = Counter(), Counter()
        request_sources, response_sources = Counter(), Counter()
        statuses = []
        for call in calls:
            request = self.one(call, "llm.call.request", "llm.request")
            response = self.one(call, "llm.call.response", "llm.response")
            request_owners[request["action_id"]] += 1
            response_owners[response["action_id"]] += 1
            attrs = call.get("attributes", {})
            if attrs.get("llm.call.request_action_id") != request["action_id"] or attrs.get("llm.call.response_action_id") != response["action_id"]:
                raise AssertionError("LLM call attributes disagree with request/response links")
            http_request = self.source(request, "llm.request.http_message")
            http_response = self.source(response, "llm.response.http_message")
            request_sources[http_request["action_id"]] += 1
            response_sources[http_response["action_id"]] += 1
            if self.exchange(http_request)["action_id"] != http_response["action_id"]:
                raise AssertionError("LLM call and HTTP exchange refer to different responses")
            self.require_context(request, response, http_request, http_response)
            status = self.http_status(http_response)
            if status < 400 or status > 599:
                raise AssertionError(f"failed-model workload did not record a provider HTTP error: {status}")
            if response.get("status") != "error" or self.http_status(response) != status:
                raise AssertionError("LLM response does not retain the provider HTTP error status")
            statuses.append(status)
        for rows, owners in ((requests, request_owners), (responses, response_owners)):
            if set(owners) != {row["action_id"] for row in rows} or any(count != 1 for count in owners.values()):
                raise AssertionError("LLM request/response has missing or duplicate call ownership")
        if any(count != 1 for counts in (request_sources, response_sources) for count in counts.values()):
            raise AssertionError("multiple LLM requests or responses share the same HTTP source")
        return {"calls": len(calls), "http_error_statuses": statuses}

    def require_ordinary_http(self, path: str, status: int) -> dict:
        if any(str(row.get("kind", "")).startswith("llm.") for row in self.actions.values()):
            raise AssertionError("ordinary HTTP request was classified as an LLM action")
        requests = [row for row in self.of_kind("http.message")
                    if row.get("attributes", {}).get("method") == "GET"
                    and row.get("attributes", {}).get("target") == path]
        if len(requests) != 1:
            raise AssertionError(f"expected one recorded GET for {path}, found {len(requests)}")
        response = self.exchange(requests[0])
        if self.http_status(response) != status:
            raise AssertionError("ordinary HTTP response status differs from the actual client result")
        return {"http_exchanges": 1, "path": path, "http_status": status}

    def exchange(self, request: dict) -> dict:
        response = self.one(request, "http.request.http_response", "http.message")
        if response.get("attributes", {}).get("http.request.action_id") != request["action_id"]:
            raise AssertionError("HTTP response request identity disagrees with its exchange link")
        self.require_context(request, response)
        return response

    def source(self, action: dict, role: str) -> dict:
        source = self.one(action, role, "http.message")
        message_id = action.get("attributes", {}).get("http.message_id")
        if message_id != source["action_id"] or source.get("attributes", {}).get("http.message_id") != message_id:
            raise AssertionError("LLM HTTP source identity disagrees with its link")
        return source

    def one(self, parent: dict, role: str, kind: str) -> dict:
        children = self.links.get((parent["action_id"], role), set())
        if len(children) != 1:
            raise AssertionError(f"expected one {role} link, found {len(children)}")
        child = self.actions.get(next(iter(children)))
        if child is None or child.get("kind") != kind:
            raise AssertionError(f"missing or wrong-kind {role} target")
        return child

    def require_context(self, *rows: dict):
        identities = []
        for row in rows:
            attrs = row.get("attributes", {})
            process = row.get("process", {}).get("process_id")
            stream = attrs.get("payload.stream_key") or attrs.get("stream_key")
            if row.get("trace_id_raw") != self.trace_id or process is None or not stream:
                raise AssertionError("request or response lacks its trace/process/connection identity")
            identities.append((process, stream))
        if len(set(identities)) != 1:
            raise AssertionError("request/response association crosses process or connection")

    def of_kind(self, kind: str) -> list[dict]:
        return [row for row in self.actions.values() if row.get("kind") == kind]

    @staticmethod
    def http_status(row: dict) -> int:
        attrs = row.get("attributes", {})
        key = "status_code" if row.get("kind") == "http.message" else "http.response.status_code"
        return int(attrs.get(key, 0))
