#!/usr/bin/env python3
"""Run a real Claude/Codex tool turn and require the selected source protocol."""

from __future__ import annotations

import argparse
from contextlib import nullcontext
import json
import os
from pathlib import Path
import secrets
import shutil
import sys
import tempfile

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.actrail_runtime import ActrailRuntime  # noqa: E402
from tests.v2.common.core import TestOutput  # noqa: E402
from tests.v2.common.llm_trace_assertion import LLMTraceAssertion  # noqa: E402
from tests.v2.regression.http_llm_sources.assertion import HttpLlmSourcesAssertion  # noqa: E402
from tests.v2.regression.http_llm_sources.websocket_assertion import WebSocketLlmSourcesAssertion  # noqa: E402
from tests.v2.regression.http_llm_sources.tls_relay import TlsRelay  # noqa: E402
from tests.v2.regression.http_llm_sources.run_e2e import HttpLlmSourcesRun, ReadOnlyContentServer  # noqa: E402


class ProtocolSourcesRun(HttpLlmSourcesRun):
    def __init__(self, options: argparse.Namespace):
        self.options = options
        self.root = Path(tempfile.mkdtemp(prefix="actrail-protocol-sources-", dir=options.output_parent))
        self.binary = Path(options.agent_binary or shutil.which(options.agent) or "")
        if not self.binary.is_file():
            raise RuntimeError(f"real {options.agent} executable unavailable")
        self.bin_dir = options.bin_dir.resolve()
        self.output = TestOutput(color_mode="never")
        self.report = {"evidence_directory": str(self.root), "agent": options.agent, "required_protocol": options.protocol, "runs": []}
        self.relay_url = None

    def run(self) -> None:
        print(f"Evidence directory: {self.root}", flush=True)
        try:
            relay_context = TlsRelay(self.options.websocket_relay_upstream, self.options.timeout) if self.options.websocket_relay_upstream else nullcontext(None)
            with relay_context as relay:
                if relay:
                    self.relay_url = relay.base_url
                    self.report["transport"] = "loopback-websocket-to-real-tls-upstream"
                    self.report["upstream"] = self.options.websocket_relay_upstream
                self.run_agent_phase(llm_enabled=True)
                self.run_agent_phase(llm_enabled=False)
            self.report["passed"] = True
        except BaseException as error:
            self.report.update(passed=False, failure=str(error) or type(error).__name__)
            raise
        finally:
            self.write_json(self.root / "report.json", self.report)

    def run_agent_phase(self, *, llm_enabled: bool) -> None:
        directory = self.root / ("llm-enabled" if llm_enabled else "llm-disabled")
        directory.mkdir(mode=0o700)
        runtime = ActrailRuntime.isolated(REPO, self.bin_dir, self.options.timeout, self.output, directory)
        with (directory / "actraild.patch.toml").open("a", encoding="utf-8") as patch:
            patch.write(
                "\n[resource_profiling.storage.database.sqlite]\n"
                f"path = {json.dumps(str(directory / 'data/resources.sqlite'))}\n"
            )
            if not llm_enabled:
                patch.write("\n[semantic_retention.l0_llm_call]\nenabled = false\n")
            if self.options.retain_transport:
                patch.write(
                    '\n[semantic_retention]\ncontent_owner = "configured_layers"\n'
                    '\n[semantic_retention.l4_payload]\nenabled = true\nstats = true\nbody_content = "retained"\n'
                )
        marker = f"PROTOCOL_SOURCES_{secrets.token_hex(8)}"
        marker_path = directory / "answer.txt"
        self.write_text(marker_path, marker + "\n")
        prompt = (
            f"Read the file {marker_path} using a tool. Reply with exactly its contents. "
            "You must actually read the file. Do not modify files or run other tasks."
        )
        prepared = False
        try:
            runtime.prepare()
            prepared = True
            result = runtime.run(runtime.control_command("launch", "--", *self.agent_command(prompt)), cwd=directory, echo=False)
            self.write_text(directory / "agent.log", result.output)
            if result.returncode != 0:
                raise AssertionError(f"real agent exited with {result.returncode}; inspect agent.log")
            check = LLMTraceAssertion(runtime, marker)
            check.require_answer_marker(result, self.options.agent)
            trace_id = check.require_trace_id(result, expected_count=1, selected_index=0)
        finally:
            if prepared:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode != 0:
                    raise RuntimeError("isolated daemon failed to stop")
        document = self.read_actions(runtime, trace_id, directory)
        closed_at = self.require_clean_trace(runtime, trace_id)
        assertion_class = WebSocketLlmSourcesAssertion if self.options.protocol == "websocket" else HttpLlmSourcesAssertion
        assertion = assertion_class(document, trace_id, closed_at, protocol=self.options.protocol)
        if llm_enabled:
            summary = assertion.require_llm(marker, minimum_successes=2, require_connection_reuse=False)
            with ReadOnlyContentServer(self.bin_dir / "actrailweb", directory / "actraild.conf", directory, self.options.timeout) as server:
                self.require_contents(server, trace_id, summary["request_ids"], marker)
        else:
            summary = assertion.require_http_only()
        self.report["runs"].append(dict(phase=directory.name, trace_id=trace_id, **summary))

    def agent_command(self, prompt: str) -> list[str | Path]:
        if self.options.agent == "codex":
            command = [
                self.binary, "exec", "--ephemeral", "--skip-git-repo-check", "--sandbox", "read-only",
                "-c", 'model_reasoning_effort="low"',
                "-c", "features.enable_request_compression=false",
            ]
            if self.options.protocol != "websocket":
                command.extend([
                    "-c", 'model_provider="openai-http"',
                    "-c", 'model_providers.openai-http={name="OpenAI", wire_api="responses", requires_openai_auth=true, supports_websockets=false}',
                ])
            elif self.relay_url:
                command.extend([
                    "-c", 'model_provider="openai-ws-relay"',
                    "-c", 'model_providers.openai-ws-relay={name="OpenAI", wire_api="responses", requires_openai_auth=true, supports_websockets=true, base_url=' + json.dumps(self.relay_url) + '}',
                ])
            if self.options.model:
                command.extend(["--model", self.options.model])
            return [*command, prompt]
        command = [
            self.binary, prompt, "--print", "--output-format", "text",
            "--no-session-persistence", "--safe-mode", "--permission-mode", "dontAsk",
            "--tools", "Read", "--allowedTools", "Read",
        ]
        if self.options.model:
            command.extend(["--model", self.options.model])
        return command


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--agent", choices=("claude", "codex"), required=True)
    parser.add_argument("--protocol", choices=("http1", "http2", "websocket"), default="http2")
    parser.add_argument("--agent-binary", type=Path)
    parser.add_argument("--model")
    parser.add_argument("--websocket-relay-upstream", help="Real HTTPS provider base URL; capture loopback WS through a verifying TLS byte relay")
    parser.add_argument("--retain-transport", action="store_true", help="Retain sensitive original transport bytes in the isolated diagnostic database")
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/release")
    parser.add_argument("--output-parent", type=Path, default=Path(tempfile.gettempdir()))
    parser.add_argument("--timeout", type=int, default=240)
    parser.add_argument("--content-max-bytes", type=int, default=8 * 1024 * 1024)
    options = parser.parse_args()
    if options.timeout < 1 or options.content_max_bytes < 1:
        parser.error("timeout and content-max-bytes must be positive")
    if options.protocol == "websocket" and options.agent != "codex":
        parser.error("WebSocket workload requires the real Codex Responses client")
    if options.websocket_relay_upstream and options.protocol != "websocket":
        parser.error("websocket-relay-upstream requires --protocol websocket")
    previous_umask = os.umask(0o077)
    try:
        run = ProtocolSourcesRun(options)
        run.run()
        print(json.dumps(run.report, ensure_ascii=False))
        return 0
    except Exception as error:
        print(f"FAILED: {error}", file=sys.stderr)
        return 1
    finally:
        os.umask(previous_umask)


if __name__ == "__main__":
    raise SystemExit(main())
