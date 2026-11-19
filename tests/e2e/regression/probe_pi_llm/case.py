"""Run Pi with scoped lifecycle management and verify finalized LLM capture."""
from __future__ import annotations

from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.core import TestRunDirectories, AgentBinaryNotFoundError, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.llm_trace_assertion import LLMTraceAssertion
from .config import ProbePiLLMConfig
from .constants import CASE_CONSTANTS
from .task import ProbePiLLMTask


class ProbePiLLMCase(TestCase):
    case_id = 'probe_pi_llm'
    description = 'Run pi through actrailctl launch and verify LLM capture'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ProbePiLLMConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("Pi launch and LLM capture") as case:
            with case.step("Check Pi availability"):
                try:
                    task = ProbePiLLMTask(self._config)
                except AgentBinaryNotFoundError as error:
                    case.skip(str(error))
                if not context.check_agent_availability(
                    "pi", task.binary, task.environment()
                ):
                    case.skip("Pi external availability check failed")

            with case.step("Capture Pi activity"):
                with ActrailRuntime.isolated(
                    self._config.repo, self._config.bin_dir,
                    self._config.command_timeout_seconds, context.output,
                    self._config.work_dir,
                ) as runtime:
                    with case.step("Start AcTrail"):
                        runtime.start_actraild()
                    with case.step("Send a request with Pi"):
                        with task.create_agent().launchWithAcTrailCtl(stream_mode=True) as agent:
                            agent.run(CASE_CONSTANTS.prompt)
                            agent.join_all(self._config.launch_timeout_seconds)
                            result = agent.results[-1]
                            if not result.succeeded:
                                raise AssertionError(result.error or "Pi task failed")
                            launch = agent.launch_result
                        if agent.returncode != 0:
                            raise AssertionError(f"Pi launch exited with {agent.returncode}")

            # Runtime 退出完成停机和排空后，检查最终请求响应。
            with case.step("Check the recorded LLM exchange"):
                LLMTraceAssertion(runtime).require_finalized_exchange(
                    launch.trace_id,
                    reply_text=result.output,
                )
        return case.result
