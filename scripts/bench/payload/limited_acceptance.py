"""Verify real TLS bpf-copy traces with demonstrated request capture limits.

This fixture accepts missing-byte evidence only when the declared body alone
exceeds the entire captured HTTP message. This is sufficient, not necessary:
headers are included in captured bytes. Other requests must have valid JSON;
ambiguous incomplete messages fail instead of being classified as complete.
"""
from __future__ import annotations

import argparse
import collections
import json
import tomllib
from pathlib import Path

from scripts.bench.payload.profile_acceptance import ProfileAcceptance


class LimitedAcceptance(ProfileAcceptance):
    def __init__(self, directory):
        super().__init__(directory)
        config = tomllib.loads((self.directory / "configs/P.resolved.toml").read_text())
        tls = config["payload"]["tls"]
        if tls["capture_backend"] != "bpf-copy" or config["seccomp_notify"]["enabled"]:
            raise RuntimeError("limited acceptance requires P bpf-copy without notify")
        self.limit = min(tls["max_operation_bytes"], 65535)
        self.segment_limit = tls["max_segment_bytes"]
        if self.limit <= 0 or self.segment_limit <= 0:
            raise RuntimeError("invalid explicit direct-copy limit")
        self.requests = []

    def request_is_limited(self, action):
        attrs = action["attributes"]
        declared = int(attrs["llm.request.payload_bytes"])
        captured = int(attrs["llm.request.raw_payload_bytes"])
        segments = int(attrs["payload.segment_count"])
        if not attrs["payload.stream_key"].startswith("tls:") or captured > segments * self.segment_limit:
            raise RuntimeError("request does not match direct-copy source or capture budget")
        limited = declared > captured
        if limited:
            if (attrs.get("http.request.body_json_state") != "invalid_or_unavailable"
                    or attrs.get("llm.request.classifier_id") != "openai-compatible-route"):
                raise RuntimeError("limited request claims unavailable JSON or lacks route evidence")
        elif attrs.get("http.request.body_json_state") != "valid":
            raise RuntimeError("complete request lacks validated JSON")
        return limited

    def expected_completeness(self, action):
        if action["kind"] != "llm.request":
            return super().expected_completeness(action)
        limited = self.request_is_limited(action)
        attrs = action["attributes"]
        self.requests.append(dict(action_id=action["action_id"], limited=limited,
            declared_body_bytes=int(attrs["llm.request.payload_bytes"]),
            captured_http_bytes=int(attrs["llm.request.raw_payload_bytes"]),
            segments=int(attrs["payload.segment_count"])))
        return "capture_limited" if limited else "complete"

    def expected_request_content_state(self, action):
        return "unavailable" if self.request_is_limited(action) else "none"

    def trace(self, sample, turns):
        if sample["mode"] != "P":
            raise RuntimeError("use standard ProfileAcceptance for C")
        self.requests = []
        result = super().trace(sample, turns)
        if not any(row["limited"] for row in self.requests):
            raise RuntimeError("no actual limited request was exercised")
        graph = json.loads((self.directory / f"actions-P-{result['trace_id']}.json").read_text())
        for role in ("llm.call.request", "llm.call.response"):
            parents = collections.Counter(link["parent_action_id"] for link in graph["links"]
                                          if link["role"] == role and link["valid"])
            if len(parents) != turns or set(parents.values()) != {1}:
                raise RuntimeError("long-request calls do not have one-to-one request/response links")
        result.update(operation_limit_bytes=self.limit, segment_limit_bytes=self.segment_limit,
                      limited_requests=sum(row["limited"] for row in self.requests),
                      requests=self.requests)
        return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    LimitedAcceptance(parser.parse_args().directory).run()
