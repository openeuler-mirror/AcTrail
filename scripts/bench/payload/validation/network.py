"""Validate request facts and provider evidence over real local HTTPS connections."""

from __future__ import annotations

import argparse
import http.client
import http.server
import json
import os
import ssl
import subprocess
import sys
import threading
import time
import tomllib
from pathlib import Path
from urllib.parse import parse_qs, urlsplit

from scripts.bench.payload.benchmark import BenchmarkLock, HERE, ROOT
from scripts.bench.payload.runtime import CollectionRuntime
from scripts.bench.payload.measurement import CommandMeasurement


class NetworkCases:
    def __init__(self):
        self.cases = []
        ordinary = {"model": "network-model", "messages": [{"role": "user", "content": "Hello"}]}
        self.add("chat", ordinary)
        self.add("background", {"model": "network-model", "messages": [
            {"content": [{"text": "You are a TITLE GENERATOR for a thread title.", "type": "text"}], "role": "system"},
            {"role": "user", "content": "large history " * 16384},
        ]}, background="title_generation")
        self.add("background_parts", {"messages": [{"role": "system", "content": [
            "conversation", {"input": "summarizer"}]}], "model": "network-model"}, background="conversation_summary")
        self.add("named_model", {"model": None, "model_name": "named-model", "tools": None,
                                "messages": [{"content": "Hi", "role": ""}]}, model="named-model")
        self.add("invalid_named_model", {"model_name": None, "provider_model_name": "ignored-model",
                                        **ordinary})
        self.add("large_ignored", {"unused": {"arguments": ["opaque " * 32768]}, **ordinary})
        self.add("duplicate_role", b'{"model":"network-model","messages":[{"content":"conversation summarizer","role":"system","role":"user"}]}')
        self.add("duplicate_content", b'{"model":"network-model","messages":[{"role":"system","content":"conversation summarizer","content":"ordinary"}]}')
        self.add("duplicate_messages", b'{"model":"network-model","messages":[{"role":"system","content":"conversation summarizer"}],"messages":[]}')
        self.add("duplicate_model", b'{"messages":[],"model":"ignored-model","model":"network-model"}')
        self.add("route_only", {"messages": []}, model=None, request_modes=["P"], response="ordinary")
        for name, value in (
            ("invalid_utf8", b'"\xff"'), ("surrogate", b'"\\ud800"'),
            ("escape", b'"\\q"'), ("number", b'01'), ("number_range", b'1e999'),
            ("depth", b'[' * 130 + b'0' + b']' * 130),
        ):
            self.add(name, b'{"model":"network-model","messages":[],"unused":' + value + b'}', valid=False)
        self.add("trailing_json", json.dumps(ordinary).encode() + b' {}', valid=False)
        self.add("valid_unicode", {"unused": "😀\n", **ordinary})
        self.add("ordinary_sse", ordinary, response="ordinary")
        self.add("responses", ordinary, response="responses")
        self.add("structured", ordinary, response="structured")
        self.add("partial_response", ordinary, partial=True)

    def add(self, name, body, *, valid=True, model="network-model", background=None,
            response="chat", partial=False, request_modes=None):
        self.cases.append({"name": name, "body": body if isinstance(body, bytes) else json.dumps(body).encode(),
                           "valid": valid, "model": model, "background": background,
                           "response": response, "partial": partial,
                           "request_modes": request_modes if request_modes is not None else ["P", "C"]})

    @staticmethod
    def response(case):
        kind = case["response"]
        if kind == "ordinary":
            events = [("heartbeat", {"sequence": 1, "temperature": 22}), ("done", {"status": "ok"})]
        elif kind == "responses":
            events = [
                ("response.created", {"type": "response.created", "response": {"id": "resp-network", "model": "network-model"}}),
                ("response.output_text.delta", {"type": "response.output_text.delta", "delta": "NETWORK_CONTENT"}),
                ("response.output_item.done", {"type": "response.output_item.done", "item": {
                    "type": "function_call", "call_id": "call-network", "name": "network_tool", "arguments": '{"value":7}'}}),
                ("response.completed", {"type": "response.completed", "response": {"id": "resp-network", "model": "network-model",
                    "status": "completed", "usage": {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18}}}),
            ]
        elif kind == "structured":
            events = [("metadata", {"model": "network-model"}),
                      ("output", {"response": "NETWORK_CONTENT", "reasoning_content": "NETWORK_REASONING"}),
                      ("token_usage", {"input_tokens": 11, "output_tokens": 7, "total_tokens": 18}),
                      ("done", {"finish_reason": "stop"})]
        else:
            events = [(None, {"model": "network-model", "choices": [{"delta": {"content": "NETWORK_CONTENT"}}]})]
            if not case["partial"]:
                events.append((None, {"model": "network-model", "choices": [{"delta": {}, "finish_reason": "stop"}],
                                      "usage": {"prompt_tokens": 11, "completion_tokens": 7, "total_tokens": 18}}))
        return "".join((f"event: {event}\n" if event else "") + f"data: {json.dumps(value)}\n\n"
                       for event, value in events).encode()


class NetworkServer:
    def __init__(self, out, cases, response_builder=NetworkCases.response):
        self.cases = {case["name"]: case for case in cases}
        self.response_builder = response_builder
        self.cert, self.key = out / "cert.pem", out / "key.pem"
        subprocess.run(["openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes",
                        "-keyout", str(self.key), "-out", str(self.cert), "-days", "1",
                        "-subj", "/CN=localhost", "-addext", "subjectAltName=IP:127.0.0.1"],
                       check=True, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=30)
        owner = self

        class Handler(http.server.BaseHTTPRequestHandler):
            protocol_version = "HTTP/1.1"

            def do_POST(self):
                case = owner.cases[parse_qs(urlsplit(self.path).query)["case"][0]]
                body = self.rfile.read(int(self.headers["Content-Length"]))
                if body != case["body"]:
                    self.send_error(400)
                    return
                response = owner.response_builder(case)
                self.send_response(200)
                self.send_header("Content-Type", "text/event-stream")
                self.send_header("Content-Length", str(len(response) + (100 if case["partial"] else 0)))
                self.send_header("Connection", "close")
                self.end_headers()
                chunks = response.splitlines(keepends=True) if case.get("split_events") else [response]
                for index, chunk in enumerate(chunks):
                    self.wfile.write(chunk)
                    self.wfile.flush()
                    if index + 1 < len(chunks):
                        time.sleep(case["event_pause_seconds"])
                self.close_connection = True

            def log_message(self, *_):
                pass

        self.server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(self.cert, self.key)
        self.server.socket = context.wrap_socket(self.server.socket, server_side=True)
        self.port = self.server.server_port
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)

    def start(self):
        self.thread.start()

    def stop(self):
        self.server.shutdown()
        self.server.server_close()
        self.thread.join(timeout=5)


class NetworkAcceptance:
    def __init__(self, out: Path, bin_dir: Path):
        self.out, self.bin_dir = out.resolve(), bin_dir.resolve()
        self.settings = tomllib.loads((HERE / "configs/benchmark.toml").read_text())

    def run(self):
        self.out.mkdir(parents=True, exist_ok=False)
        try:
            self._run()
        except BaseException as error:
            (self.out / "acceptance.json").write_text(json.dumps({"status": "failed", "error": str(error)}, indent=2) + "\n")
            raise

    def _run(self):
        cases = NetworkCases().cases
        fixture = self.out / "wire"
        fixture.mkdir()
        for case in cases:
            body_file = fixture / f"{case['name']}.request.bin"
            response_file = fixture / f"{case['name']}.response.bin"
            body_file.write_bytes(case["body"])
            response_file.write_bytes(NetworkCases.response(case))
            case["body_file"], case["response_file"] = str(body_file), str(response_file)
        with BenchmarkLock(Path("/run/lock/actrail-v2-regression.lock"), 5):
            CollectionRuntime.require_stopped()
            server = NetworkServer(self.out, cases)
            reports = []
            try:
                server.start()
                for mode in ("P", "C"):
                    reports.append(self.scenario(mode, server, cases))
            finally:
                server.stop()
        self.verify_providers(self.out, require_success=False)
        (self.out / "acceptance.json").write_text(json.dumps({"status": "passed", "scenarios": reports}, indent=2) + "\n")

    @staticmethod
    def verify_providers(out, require_success=True):
        if require_success and json.loads((out / "acceptance.json").read_text())["status"] != "passed":
            raise RuntimeError("provider evidence requires a successful real-network run")
        evidence = []
        for mode in ("P", "C"):
            graph = json.loads((out / f"actions-{mode}.json").read_text())
            NetworkAcceptance.verify_cases(mode, graph, json.loads((out / f"client-{mode}.json").read_text())["cases"])
            by_id = {action["action_id"]: action for action in graph["actions"]}
            providers = set()
            count = 0
            for action in graph["actions"]:
                if action["kind"] != "llm.response":
                    continue
                attrs = action["attributes"]
                provider = attrs["llm.response.provider_id"]
                providers.add(provider)
                structured = provider == "structured-json-sse"
                responses = provider == "openai-responses"
                if int(attrs["llm.response.chunk_count"]) != (2 if structured else 1):
                    raise RuntimeError(f"{mode}/{provider}: chunk evidence mismatch")
                if ("llm.response.reasoning_text" in attrs) != (mode == "C" and structured):
                    raise RuntimeError(f"{mode}/{provider}: reasoning retention mismatch")
                if mode == "C" and (attrs.get("llm.response.content_text") != "NETWORK_CONTENT"
                        or (structured and attrs.get("llm.response.reasoning_text") != "NETWORK_REASONING")):
                    raise RuntimeError(f"{mode}/{provider}: incorrect response text")
                if ("llm.response.tool_calls_json" in attrs) != (mode == "C" and responses):
                    raise RuntimeError(f"{mode}/{provider}: tool JSON retention mismatch")
                links = [link for link in graph["links"] if link["parent_action_id"] == action["action_id"]
                         and link["role"] == "llm.response.tool_call" and link["valid"]]
                if len(links) != (1 if mode == "C" and responses else 0):
                    raise RuntimeError(f"{mode}/{provider}: tool declaration relationship mismatch")
                if links:
                    calls = json.loads(attrs["llm.response.tool_calls_json"])
                    tool = by_id[links[0]["child_action_id"]]
                    if (len(calls) != 1 or calls[0].get("id") != "call-network"
                            or calls[0].get("function", {}).get("name") != "network_tool"
                            or json.loads(calls[0]["function"]["arguments"]) != {"value": 7}
                            or calls[0]["function"]["arguments_json"] != {"value": 7}
                            or tool["kind"] != "llm.tool_call" or tool["process"] != action["process"]
                            or tool["attributes"].get("llm.tool_call.response_action_id") != action["action_id"]
                            or tool["attributes"].get("llm.tool_call.id") != "call-network"
                            or tool["attributes"].get("llm.tool_call.name") != "network_tool"):
                        raise RuntimeError(f"{mode}/{provider}: tool identity or arguments mismatch")
                if attrs.get("llm.response.done") == "true":
                    for field, tokens in (("prompt_tokens", 11), ("completion_tokens", 7), ("total_tokens", 18)):
                        expected = str(tokens) if mode == "C" else None
                        if attrs.get(f"llm.response.{field}") != expected:
                            raise RuntimeError(f"{mode}/{provider}: {field} mismatch")
                count += 1
            if providers != {"openai-compatible", "openai-responses", "structured-json-sse"}:
                raise RuntimeError(f"{mode}: missing provider coverage: {providers}")
            evidence.append({"mode": mode, "providers": sorted(providers), "responses": count})
        (out / "provider-acceptance.json").write_text(json.dumps({"status": "passed", "traces": evidence}, indent=2) + "\n")

    def scenario(self, mode, server, cases):
        print(f"[network/{mode}]", flush=True)
        runtime = CollectionRuntime(self.out / f"r-{mode}", self.bin_dir, HERE / f"configs/{mode}.toml", self.settings)
        manifest = self.out / f"client-{mode}.json"
        manifest.write_text(json.dumps({"port": server.port, "cert": str(server.cert),
                                      "cases": [{k: v for k, v in case.items() if k != "body"} for case in cases]}))
        try:
            runtime.start()
            mark = runtime.mark()
            client_directory = self.out / f"client-{mode}"
            client_directory.mkdir()
            env = dict(os.environ, PYTHONPATH=str(ROOT))
            CommandMeasurement(90).run(runtime.launch([sys.executable, "-m", "scripts.bench.payload.validation.network",
                                                      "--client", str(manifest)]), client_directory, env)
            collection = runtime.drain(mark)
            trace_id = collection["traces"][0][0]
            output = subprocess.run([str(self.bin_dir / "actrailviewer"), "--config", str(runtime.config),
                                     "--output-format", "json", "actions", "--trace-id", str(trace_id)],
                                    check=True, text=True, capture_output=True, timeout=30).stdout
            (self.out / f"actions-{mode}.json").write_text(output)
            graph = json.loads(output)
            checked = self.verify_cases(mode, graph, cases)
            return {"mode": mode, "trace": trace_id, "health": collection["traces"][0][2], "cases": checked}
        finally:
            runtime.stop()

    @staticmethod
    def verify_cases(mode, graph, cases):
        by_id = {action["action_id"]: action for action in graph["actions"]}
        paths = {f"/v1/chat/completions?case={case['name']}": case["name"] for case in cases}
        response_groups = {case["name"]: [] for case in cases}
        for response in graph["actions"]:
            if response["kind"] != "llm.response":
                continue
            request = by_id.get(response["attributes"].get("http.request.action_id"))
            if request is not None:
                if (request["kind"] != "http.message" or request["attributes"].get("http.operation") != "request"
                        or request["process"] != response["process"]):
                    raise RuntimeError(f"{mode}: response has an invalid HTTP request identity")
                path = request["attributes"].get("target")
            else:
                parents = [link for link in graph["links"] if link["child_action_id"] == response["action_id"]
                           and link["role"] == "llm.call.response" and link["valid"]]
                if len(parents) != 1:
                    raise RuntimeError(f"{mode}: response lacks a unique request grouping")
                siblings = [link for link in graph["links"] if link["parent_action_id"] == parents[0]["parent_action_id"]
                            and link["role"] == "llm.call.request" and link["valid"]]
                if len(siblings) != 1:
                    raise RuntimeError(f"{mode}: response lacks a unique request")
                request = by_id[siblings[0]["child_action_id"]]
                if request["kind"] != "llm.request" or request["process"] != response["process"]:
                    raise RuntimeError(f"{mode}: response request identity mismatch")
                path = request["attributes"].get("url.path")
            name = paths.get(path)
            if name is None:
                raise RuntimeError(f"{mode}: unexpected or unassigned LLM response")
            response_groups[name].append(response)
        checked = []
        for case in cases:
            name = case["name"]
            path = f"/v1/chat/completions?case={name}"
            requests = [a for a in graph["actions"] if a["kind"] == "llm.request" and a["attributes"].get("url.path") == path]
            expected = 1 if mode in case.get("request_modes", ["P", "C"]) else 0
            if len(requests) != expected:
                raise RuntimeError(f"{mode}/{name}: request admission mismatch")
            responses = response_groups[name]
            if len(responses) != (0 if case["response"] == "ordinary" else 1):
                raise RuntimeError(f"{mode}/{name}: response admission mismatch")
            attrs = {}
            if requests:
                request = requests[0]
                attrs = request["attributes"]
                if ((attrs.get("http.request.body_json_state") == "valid") != case["valid"]
                        or attrs.get("llm.request.model") != case["model"]
                        or attrs.get("llm.request.background_kind") != case["background"]):
                    raise RuntimeError(f"{mode}/{name}: request metadata mismatch")
                parents = [link for link in graph["links"] if link["child_action_id"] == request["action_id"]
                           and link["role"] == "llm.call.request" and link["valid"]]
                if len(parents) != 1:
                    raise RuntimeError(f"{mode}/{name}: missing unique valid request grouping")
                call = by_id[parents[0]["parent_action_id"]]
                if call["kind"] != "llm.call" or call["process"] != request["process"]:
                    raise RuntimeError(f"{mode}/{name}: invalid request grouping identity")
                links = [link for link in graph["links"] if link["parent_action_id"] == call["action_id"]
                         and link["role"] == "llm.call.response" and link["valid"]]
                if len(links) != len(responses) or {link["child_action_id"] for link in links} != {a["action_id"] for a in responses}:
                    raise RuntimeError(f"{mode}/{name}: response binding mismatch")
            for response in responses:
                fields = response["attributes"]
                if case["partial"]:
                    if (response["completeness"] != "partial" or response["status"] != "error"
                            or fields.get("llm.response.done") == "true"):
                        raise RuntimeError(f"{mode}: transport truncation completion mismatch")
                elif (response["status"] != "success" or fields.get("llm.response.done") != "true"
                      or response["completeness"] != "complete" or not response["end_time_unix_nanos"]):
                    raise RuntimeError(f"{mode}/{name}: response completion missing")
                if ("llm.response.content_text" in fields) != (mode == "C"):
                    raise RuntimeError(f"{mode}/{name}: response content retention mismatch")
                if not case["partial"] and ("llm.response.completion_tokens" in fields) != (mode == "C"):
                    raise RuntimeError(f"{mode}/{name}: usage retention mismatch")
            checked.append({"case": name, "requests": len(requests), "json_valid": case["valid"], "responses": len(responses),
                            "classifier": attrs.get("llm.request.classifier_id"), "background": case["background"]})
        return checked

    @staticmethod
    def client(path):
        manifest = json.loads(path.read_text())
        context = ssl.create_default_context(cafile=manifest["cert"])
        for case in manifest["cases"]:
            connection = http.client.HTTPSConnection("127.0.0.1", manifest["port"], context=context, timeout=10)
            try:
                path = case.get("request_path", f"/v1/chat/completions?case={case['name']}")
                connection.request("POST", path, Path(case["body_file"]).read_bytes(), {"Content-Type": "application/json"})
                response = connection.getresponse()
                if response.status != 200:
                    raise RuntimeError(f"server rejected {case['name']}")
                try:
                    body = response.read()
                    if case["partial"]:
                        raise RuntimeError("expected a truncated HTTP response")
                except http.client.IncompleteRead as error:
                    if not case["partial"]:
                        raise
                    body = error.partial
                if body != Path(case["response_file"]).read_bytes():
                    raise RuntimeError(f"response bytes mismatch: {case['name']}")
                print(case["name"], "passed", flush=True)
            finally:
                connection.close()


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--client", type=Path)
    parser.add_argument("--verify-existing", type=Path)
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    args = parser.parse_args()
    if args.verify_existing:
        NetworkAcceptance.verify_providers(args.verify_existing)
    elif args.client:
        NetworkAcceptance.client(args.client)
    elif args.out:
        NetworkAcceptance(args.out, args.bin_dir).run()
    else:
        parser.error("--out or --client is required")
