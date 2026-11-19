"""Observe real rejected model requests and ordinary HTTP requests."""
from __future__ import annotations

import json
import os
from pathlib import Path
import shutil
import sys
import tempfile

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions, ToolAccess
from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import TestCase, TestRunDirectories, TestResult, TestStatus
from tests.e2e.common.core.loopback_port import resolve_test_port
from tests.e2e.common.llm_trace_assertion import LLMTraceAssertion
from tests.e2e.common.process import SubprocessRunner
from tests.e2e.common.testing_env import AgentBinaryDiscovery

from .assertion import NetworkRequestCoverageAssertion
from .config import NetworkRequestCoverageConfig


class NetworkRequestCoverageCase(TestCase):
    case_id = "network_request_coverage"
    description = "Record rejected LLM requests and ordinary HTTP requests"

    def __init__(self, directories: TestRunDirectories) -> None:
        self._config = NetworkRequestCoverageConfig.from_environment(directories)
        self._root = Path(tempfile.mkdtemp(prefix="network-", dir=directories.work_dir))
        self._report = {}

    def run(self, test_context) -> TestResult:
        if os.geteuid() != 0:
            return TestResult(TestStatus.SKIPPED, "Root is required for network capture")
        for name in ("actraild", "actrailctl", "actrailviewer"):
            if not os.access(self._config.bin_dir / name, os.X_OK):
                return TestResult(TestStatus.SKIPPED, f"Required executable is unavailable: {name}")
        results = {}
        launches = {}
        progress_labels = {
            "failed_llm": "Capture failed model requests",
            "ordinary_http": "Capture ordinary HTTP requests",
            "capture": "Run the observation service",
        }
        runtime = ActrailRuntime.isolated(
            self._config.repo, self._config.bin_dir, self._config.timeout_seconds,
            test_context.output, self._root,
            launch_options=LaunchOptions(tool_access=ToolAccess.NONE, max_turns=self._config.max_turns),
        )
        test_context.report_progress("Prepare request capture", f"Evidence directory: {self._root}")
        try:
            with runtime:
                runtime.prepare()
                for name, workload in (
                    ("failed_llm", lambda: self._run_failed_llm(test_context)),
                    ("ordinary_http", lambda: self._run_ordinary_http(runtime)),
                ):
                    test_context.report_progress(progress_labels[name])
                    try:
                        outcome = workload()
                        if isinstance(outcome, TestResult):
                            results[name] = outcome
                        else:
                            launches[name] = outcome
                    except Exception as error:
                        results[name] = TestResult(TestStatus.FAILED, str(error))
            for name, evidence in launches.items():
                try:
                    trace_id = evidence["trace_id"]
                    assertion = NetworkRequestCoverageAssertion(self._read_actions(runtime, trace_id), trace_id)
                    if name == "failed_llm":
                        observed = assertion.require_failed_llm()
                    else:
                        observed = assertion.require_ordinary_http(evidence["path"], evidence["http_status"])
                    self._report[name] = {**evidence, **observed}
                    results[name] = TestResult(TestStatus.PASSED, json.dumps(observed))
                except Exception as error:
                    results[name] = TestResult(TestStatus.FAILED, str(error))
        except Exception as error:
            results["capture"] = TestResult(TestStatus.FAILED, str(error))
        finally:
            for name, result in results.items():
                self._report.setdefault(name, {}).update(status=result.status.value, message=result.message)
                test_context.report_progress(
                    f"{progress_labels[name]}: {result.status.value}", result.message,
                )
            self._write_json(self._root / "report.json", self._report)
        return TestResult(TestStatus.COMPOSITE, self.description, results)

    def _run_failed_llm(self, test_context):
        discovery = AgentBinaryDiscovery(self._config.repo)
        binary = self._config.xiaoo_binary or discovery.resolve("XIAOO_E2E_BINARY", "xiaoo")
        if binary is None or not discovery.is_executable(binary):
            return TestResult(TestStatus.SKIPPED, "XiaoO executable is unavailable")
        environment = discovery.environment(binary)
        if not test_context.check_agent_availability("xiaoo", binary, environment):
            return TestResult(TestStatus.SKIPPED, "XiaoO default-model availability check failed")
        task = AgentFactory.xiaoo(
            binary=binary, environment=environment, cwd=self._root,
            arguments=("--model", self._config.error_model),
        )
        with task.launchWithAcTrailCtl(stream_mode=False) as agent:
            agent.run(self._config.prompt)
            agent.join_all(self._config.timeout_seconds)
            result = agent.results[-1]
            trace_id = agent.launch_result.trace_id
        self._write_text(self._root / "failed-llm.log", result.output + (result.error or "") + agent.diagnostics)
        self._report["failed_llm"] = {"trace_id": trace_id, "model": self._config.error_model}
        if result.succeeded:
            return TestResult(TestStatus.SKIPPED, "Provider accepted the configured error model; rejected-request scenario was not established")
        return {"trace_id": trace_id, "model": self._config.error_model}

    def _run_ordinary_http(self, runtime):
        curl = shutil.which("curl")
        if curl is None:
            return TestResult(TestStatus.SKIPPED, "curl is unavailable")
        body_file = self._root / "ordinary-http-body.txt"
        port = resolve_test_port("NETWORK_REQUEST_COVERAGE_E2E_HTTP_PORT")
        path = "/help"
        url = f"http://127.0.0.1:{port}{path}"
        entry = self._config.repo / "tests/e2e/common/test_suites/local_maas_server/server.py"
        server = SubprocessRunner().start([
            sys.executable, str(entry), "replay", "--scenario", "finite-sequence",
            "--disable-https", "--http-bind-host", "127.0.0.1",
            "--http-bind-port", str(port),
        ], cwd=self._config.repo)
        try:
            server.wait_for_output("Local MaaS server is ready", timeout=self._config.timeout_seconds)
            return self._capture_ordinary_http(runtime, curl, url, path, body_file)
        finally:
            result = server.terminate()
            self._write_text(self._root / "local-maas.log", result.stdout + result.stderr)

    def _capture_ordinary_http(self, runtime, curl: str, url: str, path: str, body_file: Path):
        result = runtime.run(runtime.control_command(
            "launch", "--", curl, "--noproxy", "*", "--silent", "--show-error",
            "--max-time", str(self._config.timeout_seconds), "--output", str(body_file),
            "--write-out", "\nHTTP_STATUS=%{http_code}\n", url,
        ), cwd=self._root, echo=False)
        self._write_text(self._root / "ordinary-http.log", result.output)
        if result.returncode != 0:
            raise AssertionError("ordinary HTTP client failed")
        statuses = [line.removeprefix("HTTP_STATUS=") for line in result.output.splitlines() if line.startswith("HTTP_STATUS=")]
        if statuses != ["200"] or not body_file.read_text(encoding="utf-8").strip():
            raise AssertionError("ordinary HTTP client did not receive the help response and its successful status")
        trace_id = LLMTraceAssertion(runtime).require_trace_id(result, expected_count=1, selected_index=0)
        evidence = {"trace_id": trace_id, "path": path, "http_status": int(statuses[0])}
        self._report["ordinary_http"] = evidence
        return evidence

    def _read_actions(self, runtime: ActrailRuntime, trace_id: int) -> dict:
        result = runtime.run_checked(runtime.viewer_command(
            "--output-format", "json", "actions", "--trace-id", str(trace_id),
        ), echo=False)
        document = json.loads(result.stdout)
        self._write_json(self._root / f"trace-{trace_id}-actions.json", document)
        return document

    @staticmethod
    def _write_text(path: Path, text: str):
        path.write_text(text, encoding="utf-8")
        path.chmod(0o600)

    @classmethod
    def _write_json(cls, path: Path, value: dict):
        cls._write_text(path, json.dumps(value, ensure_ascii=False, indent=2) + "\n")
