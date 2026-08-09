from __future__ import annotations

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import AgentBinaryNotFoundError, TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.mcp_test_support import McpTraceAssertion

from .config import ProbeCodexMcpConfig
from .task import ProbeCodexMcpTask


class ProbeCodexMcpCase(TestCase):
    def __init__(self, config: ProbeCodexMcpConfig) -> None:
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        task: ProbeCodexMcpTask | None = None
        try:
            runtime = ActrailRuntime.isolated(
                self._config.repo,
                self._config.bin_dir,
                self._config.command_timeout_seconds,
                test_context.output,
                self._config.work_dir,
            )
            try:
                task = ProbeCodexMcpTask(self._config, runtime)
            except AgentBinaryNotFoundError as error:
                return TestResult(TestStatus.SKIPPED, str(error))
            if not test_context.check_agent_availability(
                "codex",
                task.binary,
                task.environment(),
            ):
                return TestResult(
                    TestStatus.SKIPPED,
                    "Codex external availability check failed",
                )

            lifecycle = runtime.prepare()
            results["runtime_lifecycle"] = TestResult(
                TestStatus.PASSED,
                f"{len(lifecycle)} lifecycle commands completed",
            )
            launch = task.run()
            if launch.returncode != 0:
                raise AssertionError(
                    f"actrailctl launch exited with {launch.returncode}\n"
                    f"{launch.output[-4000:]}"
                )
            results["launch"] = TestResult(
                TestStatus.PASSED,
                "actrailctl launch and Codex exited successfully",
            )
            if task.tool_execution_count() == 0:
                no_tool = "NO_MCP_TOOL" in launch.stdout
                reason = (
                    "Codex did not execute "
                    f"{task.local.tool_id}"
                )
                if no_tool:
                    reason += "; Codex reported NO_MCP_TOOL"
                return TestResult(
                    TestStatus.SKIPPED,
                    reason,
                )
            execution_evidence = task.require_agent_evidence(launch)
            results["probe_execution"] = TestResult(
                TestStatus.PASSED,
                execution_evidence,
            )
            assertion = McpTraceAssertion(runtime)
            trace_id = assertion.require_trace_id(launch)
            stopped = runtime.stop()
            if stopped is None or stopped.returncode != 0:
                returncode = None if stopped is None else stopped.returncode
                raise AssertionError(
                    f"actraild safe stop exited with {returncode}"
                )
            results["runtime_stop"] = TestResult(
                TestStatus.PASSED,
                "actraild drained and stopped successfully",
            )
            summary = assertion.require_finalized_semantics(
                trace_id,
                task.expected_calls,
            )
            results["mcp_semantics"] = TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} has {summary.tool_calls} exact stdio MCP graph",
            )
            assertion.require_diagnostic(
                trace_id,
                "mcp_stdio_candidate_stream_discarded",
                "candidate_truncated",
            )
            results["mcp_diagnostic"] = TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} records recoverable stdout truncation",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "Codex stdio MCP tool-call semantic recognition",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "Codex stdio MCP tool-call semantic recognition",
                results,
            )
        finally:
            if task is not None:
                try:
                    task.close()
                except Exception as error:
                    results["cleanup"] = TestResult(TestStatus.FAILED, str(error))
            if runtime is not None:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode != 0:
                    results["runtime_stop"] = TestResult(
                        TestStatus.FAILED,
                        f"actraild stop exited with {stopped.returncode}",
                    )
