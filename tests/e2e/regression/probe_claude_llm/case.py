# Claude 回归流程：检查可用性、运行真实任务、验证停机后的观测结果。
from __future__ import annotations

from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.core import TestRunDirectories, AgentBinaryNotFoundError, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.llm_trace_assertion import LLMTraceAssertion

from .config import ProbeClaudeLLMConfig
from .constants import CASE_CONSTANTS
from .task import ProbeClaudeLLMTask


class ProbeClaudeLLMCase(TestCase):
    case_id = 'probe_claude_llm'
    description = 'Run Claude through actrailctl launch and verify LLM capture'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ProbeClaudeLLMConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("Claude launch and LLM capture") as case:
            with case.step("Check Claude availability"):
                try:
                    task = ProbeClaudeLLMTask(self._config)
                except AgentBinaryNotFoundError as error:
                    case.skip(str(error))
                if not context.check_agent_availability(
                    "claude", task.binary, task.environment()
                ):
                    case.skip("Claude external availability check failed")

            with case.step("Capture Claude activity"):
                with ActrailRuntime(
                    self._config.repo,
                    self._config.bin_dir,
                    self._config.command_timeout_seconds,
                    context.output,
                ) as runtime:
                    with case.step("Start AcTrail"):
                        runtime.start_actraild()
                    with case.step("Send a request with Claude"):
                        launch = task.run()
                        if launch.returncode != 0:
                            raise AssertionError(
                                f"actrailctl launch exited with {launch.returncode}\n"
                                f"{launch.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}"
                            )

            # Runtime 退出完成停机和排空后，读取最终落盘的请求与响应。
            with case.step("Check the recorded LLM exchange"):
                LLMTraceAssertion(runtime).require_finalized_exchange(
                    launch.trace_id,
                    reply_text=launch.stdout,
                )
        return case.result
