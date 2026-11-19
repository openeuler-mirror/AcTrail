"""Verify stream compaction and the corresponding AcTrail observations."""
from __future__ import annotations

import json

from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.core import TestRunDirectories, CommonTestConfig, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.testing_env import AgentBinaryDiscovery
from .task import CompactionTask


class ContextCompactCase(TestCase):
    case_id = 'context_compact'
    description = 'Verify Claude stream compaction and AcTrail compaction observations'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = CommonTestConfig.from_environment(directories, 'CONTEXT_COMPACT')
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("Claude context compaction") as case:
            with case.step("Check Claude availability"):
                discovery = AgentBinaryDiscovery(self._config.repo)
                binary = discovery.resolve("CLAUDE_E2E_BINARY", "claude")
                if binary is None:
                    case.skip("Claude executable not found")
                if not context.check_agent_availability(
                    "claude", binary, discovery.environment(binary)
                ):
                    case.skip("Claude external availability check failed")

            with case.step("Capture the Claude session"):
                with ActrailRuntime.isolated(
                    self._config.repo,
                    self._config.bin_dir,
                    self._config.command_timeout_seconds,
                    context.output,
                    self._config.work_dir,
                ) as runtime:
                    with case.step("Start AcTrail"):
                        runtime.start_actraild()
                    with case.step("Run the Claude session"):
                        task = CompactionTask(self._config.launch_timeout_seconds)
                        trace_id = task.run(case, binary)

            # 数据只在 Runtime 停机排空之后检查。
            with case.step("Check the compaction record") as step:
                count = self._require_compaction(runtime, trace_id)
                step.message = f"trace-{trace_id}: {count} context compaction action(s)"
        return case.result

    def _require_compaction(self, runtime: ActrailRuntime, trace_id: int) -> int:
        output = runtime.run_checked(runtime.viewer_command(
            "--output-format", "json", "actions", "--trace-id", str(trace_id)
        ), echo=False).stdout
        actions = json.loads(output)["actions"]
        count = sum(action.get("kind") == "agent.context_compaction" for action in actions)
        if count != 1:
            raise AssertionError(
                f"trace-{trace_id}: expected exactly one agent.context_compaction record, found {count}"
            )
        return count
