from __future__ import annotations

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.llm_trace_assertion import LLMTraceAssertion
from tests.v2.common.test_case import TestCase, TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton

from .config import ProbeClaudeLLMConfig
from .task import ProbeClaudeLLMTask


class ProbeClaudeLLMCase(TestCase):
    def __init__(self, config: ProbeClaudeLLMConfig):
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        del test_context
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        try:
            runtime = ActrailRuntime(
                self._config.repo,
                self._config.bin_dir,
                self._config.command_timeout_seconds,
            )
            lifecycle = runtime.prepare()
            results["runtime_lifecycle"] = TestResult(
                TestStatus.PASSED,
                f"{len(lifecycle)} lifecycle commands completed",
            )

            task = ProbeClaudeLLMTask(self._config, runtime)
            launch = task.run()
            if launch.returncode != 0:
                raise AssertionError(
                    f"actrailctl launch exited with {launch.returncode}\n"
                    f"{launch.output[-4000:]}"
                )
            results["launch"] = TestResult(
                TestStatus.PASSED,
                "actrailctl launch and Claude exited successfully",
            )

            assertion = LLMTraceAssertion(
                runtime,
                task.marker,
                self._config.drain_attempts,
                self._config.drain_interval_seconds,
            )
            assertion.require_answer_marker(launch, "Claude")
            results["answer_marker"] = TestResult(
                TestStatus.PASSED,
                f"Claude stdout answer contains {task.marker}",
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
                "Claude launch and LLM capture passed",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "Claude launch and LLM capture failed",
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
