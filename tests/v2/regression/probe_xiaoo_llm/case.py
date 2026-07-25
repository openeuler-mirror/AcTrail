from __future__ import annotations

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.errors import AgentBinaryNotFoundError
from tests.v2.common.llm_trace_assertion import LLMTraceAssertion
from tests.v2.common.test_case import TestCase, TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton

from .config import ProbeXiaooLLMConfig
from .task import ProbeXiaooLLMTask


class ProbeXiaooLLMCase(TestCase):
    def __init__(self, config: ProbeXiaooLLMConfig):
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        try:
            runtime = ActrailRuntime(
                self._config.repo,
                self._config.bin_dir,
                self._config.command_timeout_seconds,
                test_context.output,
            )
            try:
                task = ProbeXiaooLLMTask(self._config, runtime)
            except AgentBinaryNotFoundError as error:
                return TestResult(TestStatus.SKIPPED, str(error))
            if not test_context.check_agent_availability(
                "xiaoo",
                task.binary,
                task.environment(),
            ):
                return TestResult(
                    TestStatus.SKIPPED,
                    "xiaoO external availability check failed",
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
                "actrailctl launch and xiaoO exited successfully",
            )

            assertion = LLMTraceAssertion(
                runtime,
                task.marker,
                self._config.drain_attempts,
                self._config.drain_interval_seconds,
            )
            assertion.require_answer_marker(launch, "xiaoO")
            results["answer_marker"] = TestResult(
                TestStatus.PASSED,
                f"xiaoO stdout answer contains {task.marker}",
            )

            trace_id = assertion.require_trace_id(
                launch,
                expected_count=1,
                selected_index=0,
            )
            request_count, response_count = assertion.wait_and_require_exchange(trace_id)
            results["llm_exchange"] = TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} has {request_count} paired request(s), "
                f"{response_count} response(s), and captured markers",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "xiaoO launch and LLM capture",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "xiaoO launch and LLM capture",
                results,
            )
        finally:
            if runtime is not None:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode != 0:
                    results["runtime_stop"] = TestResult(
                        TestStatus.FAILED,
                        f"actraild stop exited with {stopped.returncode}",
                    )
