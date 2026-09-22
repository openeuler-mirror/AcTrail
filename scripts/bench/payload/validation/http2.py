"""Validate HTTP/2 retention, concurrent identities and peer resets over real TLS."""

from __future__ import annotations

import argparse
import json
import os
import subprocess
import tomllib
from pathlib import Path

from scripts.bench.payload.benchmark import BenchmarkLock, HERE, ROOT
from scripts.bench.payload.measurement import CommandMeasurement
from scripts.bench.payload.runtime import CollectionRuntime
from scripts.bench.payload.validation.http2_wire import Http2WireServer


class Http2Acceptance:
    def __init__(self, out, bin_dir, config_dir=HERE / "configs", request_padding_repeats=4096):
        self.out, self.bin_dir = out.resolve(), bin_dir.resolve()
        self.config_dir = config_dir.resolve()
        self.settings = tomllib.loads((HERE / "configs/benchmark.toml").read_text())
        if request_padding_repeats < 0:
            raise ValueError("request-padding-repeats must be nonnegative")
        self.request_padding_repeats = request_padding_repeats
        self.cases = self.make_cases(request_padding_repeats)

    @staticmethod
    def make_cases(request_padding_repeats=4096):
        cases = []
        for name in ("chat_json", "anthropic_json", "chat_sse", "ordinary_sse", "ordinary_reset", "reset_coalesced", "reset_delayed"):
            model = f"http2-{name}"
            content = f"HTTP2_CONTENT_{name}"
            reset = "reset" in name
            if name == "chat_json":
                response = json.dumps({"model": model, "choices": [{"message": {"role": "assistant", "content": content},
                    "finish_reason": "stop"}], "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}})
            elif name == "anthropic_json":
                response = json.dumps({"type": "message", "model": model, "role": "assistant",
                    "content": [{"type": "text", "text": content}], "stop_reason": "end_turn",
                    "usage": {"input_tokens": 11, "output_tokens": 7}})
            elif name.startswith("ordinary_"):
                response = 'event: heartbeat\ndata: {"temperature":22}\n\nevent: done\ndata: {"status":"ok"}\n\n'
            else:
                response = "data: " + json.dumps({"model": model, "choices": [{"delta": {"content": content}}]}) + "\n\n"
                if not reset:
                    response += "data: " + json.dumps({"model": model, "choices": [{"delta": {}, "finish_reason": "stop"}],
                        "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}}) + "\n\n"
            cases.append({"name": name, "reset": reset, "content": content, "response": response,
                          "request": {"unused": {"large": "opaque " * request_padding_repeats}, "messages": [
                              {"role": "user", "content": "Hello"}], "model": model}})
        return cases

    def run(self):
        self.out.mkdir(parents=True, exist_ok=False)
        (self.out / "cases.json").write_text(json.dumps(self.cases, indent=2))
        try:
            with BenchmarkLock(Path("/run/lock/actrail-v2-regression.lock"), 5):
                server = Http2WireServer(self.out, self.cases)
                try:
                    server.start()
                    reports = [self.scenario(mode, server) for mode in ("P", "C")]
                finally:
                    server.stop()
            result = {"status": "passed", "scenarios": reports,
                      "request_padding_repeats": self.request_padding_repeats}
        except BaseException as error:
            (self.out / "acceptance.json").write_text(json.dumps({"status": "failed", "error": str(error),
                "request_padding_repeats": self.request_padding_repeats}, indent=2))
            raise
        (self.out / "acceptance.json").write_text(json.dumps(result, indent=2) + "\n")

    def scenario(self, mode, server):
        print(f"[http2/{mode}]", flush=True)
        runtime = CollectionRuntime(self.out / f"r-{mode}", self.bin_dir, self.config_dir / f"{mode}.toml", self.settings)
        directory = self.out / f"client-{mode}"
        directory.mkdir()
        manifest = directory / "manifest.json"
        manifest.write_text(json.dumps({"port": server.port, "cert": str(server.cert), "cases": self.cases,
                                      "result": str(directory / "result.json")}))
        try:
            runtime.start()
            mark = runtime.mark()
            cpu_start = runtime.cpu.read_ms()
            measured = CommandMeasurement(40).run(runtime.launch([
                "node", str(Path(__file__).with_name("http2_client.mjs")), str(manifest)]), directory, dict(os.environ))
            collected = runtime.drain(mark)
            daemon_ms = runtime.cpu.read_ms() - cpu_start
            trace = collected["traces"][0][0]
            output = subprocess.run([str(self.bin_dir / "actrailviewer"), "--config", str(runtime.config),
                "--output-format", "json", "actions", "--trace-id", str(trace)],
                text=True, capture_output=True, check=True, timeout=30).stdout
            (self.out / f"actions-{mode}.json").write_text(output)
            diagnostics = subprocess.run([str(self.bin_dir / "actrailviewer"), "--config", str(runtime.config),
                "diagnostics", "--trace-id", str(trace)], text=True, capture_output=True, check=True, timeout=30).stdout
            (self.out / f"diagnostics-{mode}.txt").write_text(diagnostics)
            verified = self.verify(mode, json.loads(output), json.loads((directory / "result.json").read_text()))
            return {"mode": mode, "trace": trace, "traces": collected["traces"], "verified": verified,
                    "measurement": measured, "daemon_cpu_ms": daemon_ms}
        finally:
            runtime.stop()

    def verify(self, mode, graph, client):
        if client["alpn"] != "h2" or len(client["streams"]) != len(self.cases):
            raise RuntimeError("missing actual HTTP/2 client streams")
        streams = {item["name"]: str(item["stream_id"]) for item in client["streams"]}
        if len(set(streams.values())) != len(self.cases):
            raise RuntimeError("duplicate actual HTTP/2 stream identity")
        requests = [a for a in graph["actions"] if a["kind"] == "llm.request"]
        responses = [a for a in graph["actions"] if a["kind"] == "llm.response"]
        if len(requests) != len(self.cases) or len(responses) != len(self.cases) - 2:
            raise RuntimeError(f"{mode}: request/response counts {len(requests)}/{len(responses)}")
        by_id = {a["action_id"]: a for a in graph["actions"]}
        connections = set()
        verified = []
        for case in self.cases:
            name, stream = case["name"], streams[case["name"]]
            candidates = [a for a in requests if a["attributes"].get("http.request.stream_id") == stream]
            if len(candidates) != 1:
                raise RuntimeError(f"{mode}/{name}: request identity mismatch")
            request = candidates[0]
            attrs = request["attributes"]
            if (attrs.get("llm.request.model") != case["request"]["model"] or attrs.get("network.protocol.version") != "h2"
                    or attrs.get("http.request.body_json_state") != "valid"
                    or (attrs.get("llm.request.content_state") == "none") != (mode == "P")):
                raise RuntimeError(f"{mode}/{name}: request classification mismatch")
            connections.add((json.dumps(request["process"], sort_keys=True), attrs["payload.stream_key"]))
            parents = [link for link in graph["links"] if link["child_action_id"] == request["action_id"]
                       and link["role"] == "llm.call.request" and link["valid"]]
            if (len(parents) != 1 or by_id[parents[0]["parent_action_id"]]["process"] != request["process"]
                    or by_id[parents[0]["parent_action_id"]]["kind"] != "llm.call"):
                raise RuntimeError(f"{mode}/{name}: unique same-process request grouping required")
            bindings = [link for link in graph["links"] if link["parent_action_id"] == parents[0]["parent_action_id"]
                        and link["role"] == "llm.call.response" and link["valid"]]
            observed = [a for a in responses if a["attributes"].get("http.response.stream_id") == stream]
            expected = 0 if name.startswith("ordinary_") else 1
            if len(observed) != expected or {a["action_id"] for a in observed} != {l["child_action_id"] for l in bindings}:
                raise RuntimeError(f"{mode}/{name}: response binding mismatch")
            for response in observed:
                fields = response["attributes"]
                if (response["process"] != request["process"] or fields.get("payload.stream_key") != attrs["payload.stream_key"]
                        or fields.get("network.protocol.version") != "h2"):
                    raise RuntimeError(f"{mode}/{name}: response stream/process mismatch")
                status, completeness = ("error", "partial") if case["reset"] else ("success", "complete")
                if (response["status"] != status or response["completeness"] != completeness
                        or fields.get("llm.response.done") != str(not case["reset"]).lower()
                        or not response["end_time_unix_nanos"]):
                    raise RuntimeError(f"{mode}/{name}: completion mismatch")
                if fields.get("llm.response.content_text") != (case["content"] if mode == "C" else None):
                    raise RuntimeError(f"{mode}/{name}: content mismatch")
                if not case["reset"]:
                    for field, tokens in (("prompt_tokens", 11), ("completion_tokens", 7), ("total_tokens", 18)):
                        retained = mode == "C" and not (name == "anthropic_json" and field == "total_tokens")
                        if fields.get(f"llm.response.{field}") != (str(tokens) if retained else None):
                            raise RuntimeError(f"{mode}/{name}: usage mismatch")
            verified.append({"case": name, "stream": stream, "responses": expected})
        if len(connections) != 1:
            raise RuntimeError("fixture must use one connection for all concurrent streams")
        frames = [a for a in graph["actions"] if a["kind"] == "http.message"
                  and a["attributes"].get("network.protocol.version") == "h2"
                  and a["attributes"].get("http.operation") == "frame"]
        if bool(frames) != (mode == "C"):
            raise RuntimeError(f"{mode}: HTTP2 detail retention mismatch")
        return verified


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--config-dir", type=Path, default=HERE / "configs",
                        help="Directory containing P.toml and C.toml configuration patches")
    parser.add_argument("--request-padding-repeats", type=int, default=4096,
                        help="Number of seven-byte padding repetitions per request (default: 4096)")
    args = parser.parse_args()
    Http2Acceptance(args.out, args.bin_dir, args.config_dir, args.request_padding_repeats).run()
