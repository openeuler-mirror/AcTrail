"""Validate provider failures in complete HTTP responses over real HTTPS."""

from __future__ import annotations

import argparse
import json
from pathlib import Path

from scripts.bench.payload.benchmark import BenchmarkLock, ROOT
from scripts.bench.payload.runtime import CollectionRuntime
from scripts.bench.payload.validation.network import NetworkAcceptance, NetworkServer


class FailureCases:
    def __init__(self, event_pause_seconds):
        self.cases = []
        for protocol, terminal in (("responses", "failed"), ("responses", "incomplete"),
                                   ("responses", "error"), ("anthropic", "error"), ("chat", "error")):
            for content in (False, True):
                self.add_failure(protocol, terminal, content)
        for protocol in ("responses", "anthropic", "chat"):
            self.add_failure(protocol, "failed" if protocol == "responses" else "error", True, trailing_done=True)
            for heartbeat in (False, True):
                for associated in (False, True):
                    events = [("heartbeat", {"temperature": 22})] if heartbeat else []
                    events.append(self.error_event(protocol, "error"))
                    self.add(f"generic_{protocol}_{heartbeat}_{associated}", events, None, associated=associated)
        self.add("responses_standalone", [self.error_event("responses", "failed")], "openai-responses")
        self.add("responses_invalid_failure", [("response.failed", {"type": "response.failed", "response": None})], None)
        for case in list(self.cases):
            self.cases.append({**case, "name": case["name"] + "_split",
                               "request_path": case["request_path"] + "_split",
                               "split_events": True, "event_pause_seconds": event_pause_seconds})

    @staticmethod
    def error_event(protocol, terminal):
        if protocol == "responses":
            if terminal == "error":
                return "error", {"type": "error", "code": "server_error", "message": "PROVIDER_FAILURE"}
            response = {"id": "resp-failed", "model": "failure-model", "status": terminal}
            if terminal == "failed":
                response["error"] = {"code": "server_error", "message": "PROVIDER_FAILURE"}
            else:
                response["incomplete_details"] = {"reason": "max_output_tokens"}
            return f"response.{terminal}", {"type": f"response.{terminal}", "response": response}
        if protocol == "anthropic":
            return "error", {"type": "error", "error": {"type": "overloaded_error", "message": "PROVIDER_FAILURE"}}
        return None, {"error": {"type": "server_error", "message": "PROVIDER_FAILURE"}}

    def add_failure(self, protocol, terminal, content, trailing_done=False):
        text = "FAILURE_CONTENT" if content else None
        if protocol == "responses":
            events = [("response.created", {"type": "response.created", "response": {
                "id": "resp-failed", "model": "failure-model", "status": "in_progress"}})]
            if content:
                events.append(("response.output_text.delta", {"type": "response.output_text.delta", "delta": text}))
            done = ("response.completed", {"type": "response.completed", "response": {"status": "completed"}})
            provider = "openai-responses"
        elif protocol == "anthropic":
            events = [("message_start", {"type": "message_start", "message": {
                "id": "msg-failed", "type": "message", "role": "assistant", "model": "failure-model", "content": []}})]
            if content:
                events.append(("content_block_delta", {"type": "content_block_delta", "index": 0,
                                                       "delta": {"type": "text_delta", "text": text}}))
            done = ("message_stop", {"type": "message_stop"})
            provider = "anthropic-messages"
        else:
            events = [(None, {"model": "failure-model", "choices": [{"delta": {
                "content": text} if content else {"role": "assistant"}}]})]
            done = (None, "[DONE]")
            provider = "openai-compatible"
        events.append(self.error_event(protocol, terminal))
        if trailing_done:
            events.append(done)
        self.add(f"{protocol}_{terminal}_{content}_{trailing_done}", events, provider, content=text)

    def add(self, name, events, provider, content=None, associated=True):
        response = "".join((f"event: {event}\n" if event else "") + "data: " +
                           (value if isinstance(value, str) else json.dumps(value)) + "\n\n"
                           for event, value in events)
        body = {"model": "failure-model", "messages": [{"role": "user", "content": "Hello"}]} if associated else {"health": True}
        route = "/v1/chat/completions" if associated else "/health"
        self.cases.append({"name": name, "body": json.dumps(body).encode(), "wire": response, "provider": provider,
                           "content": content, "associated": associated, "partial": False,
                           "request_path": f"{route}?case={name}"})

    @staticmethod
    def response(case):
        return case["wire"].encode()


class FailureAcceptance(NetworkAcceptance):
    def __init__(self, out, bin_dir, event_pause_seconds):
        super().__init__(out, bin_dir)
        self.event_pause_seconds = event_pause_seconds

    def _run(self):
        cases = FailureCases(self.event_pause_seconds).cases
        fixture = self.out / "wire"
        fixture.mkdir()
        for case in cases:
            body_file, response_file = fixture / f"{case['name']}.request.bin", fixture / f"{case['name']}.response.bin"
            body_file.write_bytes(case["body"])
            response_file.write_bytes(FailureCases.response(case))
            case["body_file"], case["response_file"] = str(body_file), str(response_file)
        with BenchmarkLock(Path("/run/lock/actrail-v2-regression.lock"), 5):
            CollectionRuntime.require_stopped()
            server = NetworkServer(self.out, cases, FailureCases.response)
            try:
                server.start()
                reports = [self.scenario(mode, server, cases) for mode in ("P", "C")]
            finally:
                server.stop()
        if any(report["health"].lower() != "clean" for report in reports):
            raise RuntimeError("complete provider failures must not hide collection loss")
        (self.out / "acceptance.json").write_text(json.dumps({"status": "passed", "scenarios": reports}, indent=2) + "\n")

    @staticmethod
    def verify_cases(mode, graph, cases):
        by_id = {action["action_id"]: action for action in graph["actions"]}
        requests = [a for a in graph["actions"] if a["kind"] == "llm.request"]
        responses = [a for a in graph["actions"] if a["kind"] == "llm.response"]
        if len(requests) != sum(case["associated"] for case in cases):
            raise RuntimeError(f"{mode}: LLM request count mismatch")
        if len(responses) != sum(case["provider"] is not None for case in cases):
            raise RuntimeError(f"{mode}: LLM response count mismatch: {len(responses)}")
        assigned = set()
        verified = []
        for case in cases:
            path = case["request_path"]
            candidates = [r for r in requests if r["attributes"].get("url.path") == path]
            if len(candidates) != int(case["associated"]):
                raise RuntimeError(f"{mode}/{case['name']}: request admission mismatch")
            http = [a for a in graph["actions"] if a["kind"] == "http.message"
                    and a["attributes"].get("target") == path and a["attributes"].get("http.operation") == "request"]
            if len(http) != 1:
                raise RuntimeError("real HTTP request evidence missing")
            http_responses = [by_id[link["child_action_id"]] for link in graph["links"]
                              if link["parent_action_id"] == http[0]["action_id"]
                              and link["role"] == "http.request.http_response" and link["valid"]]
            if (len(http_responses) != 1 or http_responses[0]["status"] != "success"
                    or http_responses[0]["completeness"] != "complete"):
                raise RuntimeError("provider error fixture requires complete successful HTTP framing")
            bindings = []
            if candidates:
                parents = [link for link in graph["links"] if link["child_action_id"] == candidates[0]["action_id"]
                           and link["role"] == "llm.call.request" and link["valid"]]
                if len(parents) != 1:
                    raise RuntimeError("missing unique call request identity")
                call = by_id[parents[0]["parent_action_id"]]
                if call["kind"] != "llm.call" or call["process"] != candidates[0]["process"]:
                    raise RuntimeError("invalid call identity")
                bindings = [link["child_action_id"] for link in graph["links"] if link["parent_action_id"] == call["action_id"]
                            and link["role"] == "llm.call.response" and link["valid"]]
            matching = [r for r in responses if r["attributes"].get("http.request.action_id") == http[0]["action_id"]
                        or ("http.request.action_id" not in r["attributes"] and r["action_id"] in bindings)]
            if len(matching) != int(case["provider"] is not None):
                raise RuntimeError(f"{mode}/{case['name']}: response admission mismatch")
            if set(bindings) != {response["action_id"] for response in matching}:
                raise RuntimeError("failed response call binding mismatch")
            for response in matching:
                assigned.add(response["action_id"])
                attrs = response["attributes"]
                if (response["status"] != "error" or response["completeness"] != "partial"
                        or not response["end_time_unix_nanos"] or attrs.get("llm.response.done") != "true"
                        or attrs.get("llm.response.provider_id") != case["provider"]
                        or attrs.get("llm.response.chunk_count") != str(int(case["content"] is not None))
                        or response["process"] != candidates[0]["process"]):
                    raise RuntimeError(f"{mode}/{case['name']}: provider termination mismatch")
                if attrs.get("llm.response.content_text") != (case["content"] if mode == "C" else None):
                    raise RuntimeError("failure response content retention mismatch")
                if case.get("split_events") and len(attrs.get("payload.operation_ids", "").split(",")) < 3:
                    raise RuntimeError("split failure requires actual multiple TLS read operations")
                if any(key in attrs for key in ("llm.response.tool_calls_json", "llm.response.prompt_tokens",
                                                "llm.response.completion_tokens")):
                    raise RuntimeError("unexpected failure fixture tools or usage")
            verified.append({"case": case["name"], "provider": case["provider"], "responses": len(matching)})
        if assigned != {r["action_id"] for r in responses}:
            raise RuntimeError("unassigned LLM response")
        return verified


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--event-pause-seconds", type=float, default=0.01)
    args = parser.parse_args()
    if args.event_pause_seconds <= 0:
        parser.error("--event-pause-seconds must be positive")
    FailureAcceptance(args.out, args.bin_dir, args.event_pause_seconds).run()
