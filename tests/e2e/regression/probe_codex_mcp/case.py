"""Verify Codex MCP execution and finalized evidence with scoped resources."""
from __future__ import annotations

from pathlib import Path
from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions
from tests.e2e.common.core import TestRunDirectories, AgentBinaryNotFoundError, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.mcp_test_support import McpTraceAssertion
from .config import ProbeCodexMcpConfig
from .task import ProbeCodexMcpTask


class ProbeCodexMcpCase(TestCase):
    case_id = 'probe_codex_mcp'
    description = 'Run a real Codex stdio MCP call and verify its fixed semantic graph'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ProbeCodexMcpConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("Codex stdio MCP tool-call semantic recognition") as case:
            with case.step("Locate Codex"):
                try:
                    task = ProbeCodexMcpTask(self._config)
                except AgentBinaryNotFoundError as error:
                    case.skip(str(error))
            with task:
                with case.step("Check Codex availability"):
                    if not context.check_agent_availability("codex", task.binary, task.environment()):
                        case.skip("Codex external availability check failed")
                with ActrailRuntime.isolated(
                    self._config.repo, self._config.bin_dir,
                    self._config.command_timeout_seconds, context.output, self._config.work_dir,
                    operator_config_patch=Path(__file__).with_name("patch_config.toml"),
                    agent_invocation_commands=[task.native_binary],
                    launch_options=LaunchOptions(host_ebpf="auto"),
                ) as runtime:
                    with case.step("Start AcTrail"):
                        runtime.start_actraild()
                    with case.step("Ask Codex to use the MCP tool"):
                        launch, result = task.run()
                    with case.step("Check that Codex ran the MCP tool") as step:
                        if task.tool_execution_count() == 0:
                            reason = f"Codex did not execute {task.local.tool_id}"
                            if task.replied_no_tool(result.output):
                                reason += "; Codex reported NO_MCP_TOOL"
                            case.skip(reason)
                        step.message = task.require_agent_evidence()
                assertion = McpTraceAssertion(runtime)
                with case.step("Check the recorded MCP tool calls") as step:
                    summary = assertion.require_finalized_semantics(launch.trace_id, task.expected_calls)
                    step.message = f"trace-{launch.trace_id}: {summary.tool_calls} exact stdio MCP graph"
                with case.step("Check the truncated MCP stream diagnostic"):
                    assertion.require_diagnostic(
                        launch.trace_id, "mcp_stdio_candidate_stream_discarded", "candidate_truncated",
                    )
        return case.result
