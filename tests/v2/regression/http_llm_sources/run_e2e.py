#!/usr/bin/env python3
"""Exercise real Xiaoo HTTP/1 calls using refreshed, isolated daemon defaults."""

from __future__ import annotations

import argparse
import json
import os
import secrets
import shutil
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.actrail_runtime import ActrailRuntime  # noqa: E402
from tests.v2.common.core import TestOutput  # noqa: E402
from tests.v2.common.core.loopback_port import resolve_test_port  # noqa: E402
from tests.v2.common.llm_trace_assertion import LLMTraceAssertion  # noqa: E402
from tests.v2.regression.http_llm_sources.assertion import HttpLlmSourcesAssertion  # noqa: E402


class HttpLlmSourcesRun:
    def __init__(self, options: argparse.Namespace):
        self.options = options
        self.root = Path(tempfile.mkdtemp(prefix="actrail-http-llm-sources-", dir=options.output_parent))
        self.binary = Path(options.xiaoo or shutil.which("xiaoo") or "")
        if not self.binary.is_file():
            raise RuntimeError("real Xiaoo binary unavailable; set --xiaoo")
        self.bin_dir = options.bin_dir.resolve()
        self.output = TestOutput(color_mode="never")
        self.report = {"evidence_directory": str(self.root), "runs": []}

    def run(self) -> None:
        print(f"Evidence directory: {self.root}", flush=True)
        try:
            self.run_phase(llm_enabled=True)
            self.run_phase(llm_enabled=False)
            total_errors = sum(row.get("http_404s", 0) for row in self.report["runs"])
            if total_errors < self.options.min_http_errors:
                raise AssertionError(
                    f"HTTP 404/retry coverage missing: observed {total_errors}, "
                    f"required {self.options.min_http_errors}; provider did not exercise this path"
                )
            self.report["passed"] = True
        except Exception as error:
            self.report.update(passed=False, failure=str(error))
            raise
        finally:
            self.write_json(self.root / "report.json", self.report)

    def run_phase(self, *, llm_enabled: bool) -> None:
        directory = self.root / ("llm-enabled" if llm_enabled else "llm-disabled")
        directory.mkdir(mode=0o700)
        runtime = ActrailRuntime.isolated(
            REPO, self.bin_dir, self.options.timeout, self.output, directory,
        )
        patch = directory / "actraild.patch.toml"
        with patch.open("a", encoding="utf-8") as output:
            output.write(
                "\n[resource_profiling.storage.database.sqlite]\n"
                f"path = {json.dumps(str(directory / 'data/resources.sqlite'))}\n"
            )
            if not llm_enabled:
                output.write("\n[semantic_retention.l0_llm_call]\nenabled = false\n")
        launches = []
        prepared = False
        try:
            runtime.prepare()
            prepared = True
            for index in range(self.options.runs if llm_enabled else 1):
                marker = f"HTTP_SOURCES_{secrets.token_hex(8)}"
                marker_path = directory / f"answer-{index}.txt"
                marker_path.write_text(marker + "\n", encoding="utf-8")
                prompt = (
                    f"Use the file_read tool to read {marker_path}. You must actually read "
                    "the file; do not guess its contents. Then reply with exactly its "
                    "contents and nothing else. Do not modify files or use other tools."
                )
                command = runtime.control_command(
                    "launch", "--", self.binary, "--cli", "run",
                    "--tools", "file_read", "--max-turns", str(self.options.max_turns),
                    "--prompt", prompt,
                )
                if self.options.xiaoo_config:
                    command.extend(["--config", self.options.xiaoo_config.resolve()])
                result = runtime.run(command, cwd=directory, echo=False)
                self.write_text(directory / f"agent-{index}.log", result.output)
                if result.returncode != 0:
                    raise AssertionError(f"real Xiaoo launch failed: agent-{index}.log")
                check = LLMTraceAssertion(runtime, marker)
                check.require_answer_marker(result, "Xiaoo")
                trace_id = check.require_trace_id(result, expected_count=1, selected_index=0)
                launches.append((trace_id, marker))
            plain_trace = self.run_plain_http(runtime, directory)
        finally:
            if prepared:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode != 0:
                    raise RuntimeError(f"isolated daemon failed to stop: {directory}")
        config = directory / "actraild.conf"
        with ReadOnlyContentServer(self.bin_dir / "actrailweb", config, directory, self.options.timeout) as server:
            for trace_id, marker in launches:
                document = self.read_actions(runtime, trace_id, directory)
                closed_at = self.require_clean_trace(runtime, trace_id)
                assertion = HttpLlmSourcesAssertion(document, trace_id, closed_at)
                if llm_enabled:
                    result = assertion.require_llm(marker, minimum_successes=2)
                    self.require_contents(server, trace_id, result["request_ids"], marker)
                else:
                    result = assertion.require_http_only()
                self.report["runs"].append(dict(phase=directory.name, trace_id=trace_id, **result))
            closed_at = self.require_clean_trace(runtime, plain_trace)
            ordinary = HttpLlmSourcesAssertion(
                self.read_actions(runtime, plain_trace, directory), plain_trace, closed_at,
            ).require_http_only()
            self.report["runs"].append(dict(phase=directory.name, workload="ordinary-http", trace_id=plain_trace, **ordinary))

    def run_plain_http(self, runtime: ActrailRuntime, directory: Path) -> int:
        curl = shutil.which("curl")
        if not curl:
            raise RuntimeError("curl is required for ordinary HTTP observation")
        result = runtime.run(runtime.control_command(
            "launch", "--", curl, "--http1.1", "--silent", "--show-error",
            "--max-time", str(self.options.timeout), "--output", "/dev/null",
            self.options.plain_url,
        ), cwd=directory, echo=False)
        self.write_text(directory / "ordinary-http.log", result.output)
        if result.returncode != 0:
            raise AssertionError("ordinary real HTTP client failed")
        return LLMTraceAssertion(runtime).require_trace_id(result, expected_count=1, selected_index=0)

    def read_actions(self, runtime: ActrailRuntime, trace_id: int, directory: Path) -> dict:
        result = runtime.run_checked(runtime.viewer_command(
            "--output-format", "json", "actions", "--trace-id", str(trace_id),
        ), echo=False)
        document = json.loads(result.stdout)
        self.write_json(directory / f"trace-{trace_id}-actions.json", document)
        return document

    def require_clean_trace(self, runtime: ActrailRuntime, trace_id: int) -> int:
        result = runtime.run_checked(runtime.viewer_command("--output-format", "json", "traces"), echo=False)
        trace = next(row for row in json.loads(result.stdout)["traces"] if row.get("trace_id_raw") == trace_id)
        if trace.get("state") not in {"Exited", "Completed"} or trace.get("health") != "Clean":
            raise AssertionError(f"trace-{trace_id} is not cleanly terminal")
        closed_at = trace.get("exited_at_unix_nanos") or trace.get("completed_at_unix_nanos")
        if closed_at is None:
            raise AssertionError(f"trace-{trace_id} has no recorded closing time")
        return int(closed_at)

    def require_contents(self, server: ReadOnlyContentServer, trace_id: int, request_ids: list[str], marker: str) -> None:
        marker_found = False
        for action_id in request_ids:
            action = urllib.parse.quote(action_id, safe="")
            content = server.read(
                f"/api/traces/{trace_id}/actions/{action}/content/llm-request?max_bytes={self.options.content_max_bytes}"
            ).get("content")
            if not content or content.get("action_id") != action_id or content.get("truncated"):
                raise AssertionError(f"canonical request body unavailable/incomplete: {action_id}")
            body = json.loads(content["body_json"])
            if not isinstance(body, dict) or not body:
                raise AssertionError("canonical request is not a nonempty provider JSON object")
            marker_found |= marker in content["body_json"]
        if not marker_found:
            raise AssertionError("real tool-result marker missing from subsequent request body")

    @staticmethod
    def write_text(path: Path, text: str) -> None:
        path.write_text(text, encoding="utf-8")
        path.chmod(0o600)

    @classmethod
    def write_json(cls, path: Path, value: dict) -> None:
        cls.write_text(path, json.dumps(value, ensure_ascii=False, indent=2) + "\n")


class ReadOnlyContentServer:
    def __init__(self, binary: Path, config: Path, directory: Path, timeout: int):
        self.binary, self.config, self.directory, self.timeout = binary, config, directory, timeout
        self.port = resolve_test_port("ACTRAIL_HTTP_SOURCES_WEB_PORT")
        self.process = None
        self.log = None

    def __enter__(self) -> ReadOnlyContentServer:
        self.log = (self.directory / "web.log").open("w", encoding="utf-8")
        self.process = subprocess.Popen([
            str(self.binary), "--config", str(self.config), "--addr", "127.0.0.1", "--port", str(self.port),
        ], stdout=self.log, stderr=subprocess.STDOUT)
        deadline = time.monotonic() + self.timeout
        try:
            while time.monotonic() < deadline:
                if self.process.poll() is not None:
                    raise RuntimeError("read-only web process exited; inspect web.log")
                try:
                    self.read("/api/traces")
                    return self
                except (urllib.error.URLError, TimeoutError):
                    time.sleep(0.1)
            raise TimeoutError("read-only web readiness timed out")
        except BaseException:
            self.__exit__(None, None, None)
            raise

    def __exit__(self, *_args) -> None:
        if self.process is not None and self.process.poll() is None:
            self.process.terminate()
            try:
                self.process.wait(timeout=5)
            except subprocess.TimeoutExpired:
                self.process.kill()
                self.process.wait()
        if self.log is not None:
            self.log.close()

    def read(self, path: str) -> dict:
        with urllib.request.urlopen(f"http://127.0.0.1:{self.port}{path}", timeout=self.timeout) as response:
            return json.load(response)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/release")
    parser.add_argument("--xiaoo", default=os.environ.get("XIAOO_E2E_BINARY"))
    parser.add_argument("--xiaoo-config", type=Path)
    parser.add_argument("--plain-url", required=True, help="Existing real HTTP endpoint for a non-LLM GET")
    parser.add_argument("--output-parent", type=Path, default=Path(tempfile.gettempdir()))
    parser.add_argument("--runs", type=int, default=2)
    parser.add_argument("--max-turns", type=int, default=4)
    parser.add_argument("--min-http-errors", type=int, default=1, help="Minimum actual HTTP 404 responses across enabled agent runs")
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--content-max-bytes", type=int, default=8 * 1024 * 1024)
    options = parser.parse_args()
    for name in ("runs", "max_turns", "min_http_errors", "timeout", "content_max_bytes"):
        if getattr(options, name) < 1:
            parser.error(f"--{name.replace('_', '-')} must be positive")
    previous_umask = os.umask(0o077)
    try:
        run = HttpLlmSourcesRun(options)
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
