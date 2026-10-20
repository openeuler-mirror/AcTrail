from __future__ import annotations

from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import AgentBinaryNotFoundError, TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.llm_trace_assertion import LLMTraceAssertion

from .config import ProbeDcodeLLMConfig
from .task import ProbeDcodeLLMTask


class ProbeDcodeLLMCase(TestCase):
    def __init__(self, config: ProbeDcodeLLMConfig):
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        try:
            runtime = ActrailRuntime.isolated(
                self._config.repo,
                self._config.bin_dir,
                self._config.command_timeout_seconds,
                test_context.output,
                self._config.work_dir,
            )
            patch_path = self._config.work_dir.resolve() / "actraild.patch.toml"
            backend_patch = Path(__file__).with_name("actraild.patch.toml").read_text(
                encoding="utf-8"
            )
            patch_path.write_text(
                patch_path.read_text(encoding="utf-8").replace(
                    "[payload.tls]\n", backend_patch + "\n", 1
                ),
                encoding="utf-8",
            )
            try:
                task = ProbeDcodeLLMTask(self._config, runtime, test_context.output)
            except AgentBinaryNotFoundError as error:
                return TestResult(TestStatus.SKIPPED, str(error))
            test_context.report_progress(
                "agent_availability",
                "checking dcode availability",
            )
            if not task.check_availability():
                return TestResult(
                    TestStatus.SKIPPED,
                    "dcode external availability check failed",
                )

            test_context.report_progress(
                "runtime_prepare",
                "preparing actraild runtime with bpf-copy TLS capture",
            )
            lifecycle = runtime.prepare()
            results["runtime_lifecycle"] = TestResult(
                TestStatus.PASSED,
                f"{len(lifecycle)} lifecycle commands completed",
            )

            test_context.report_progress(
                "agent_launch",
                "launching dcode",
            )
            launch = task.run()
            if launch.returncode != 0:
                raise AssertionError(
                    f"actrailctl launch exited with {launch.returncode}\n"
                    f"{launch.output[-4000:]}"
                )
            results["launch"] = TestResult(
                TestStatus.PASSED,
                "actrailctl launch and dcode exited successfully",
            )

            test_context.report_progress(
                "launch_validation",
                "validating answer marker and trace id",
            )
            assertion = LLMTraceAssertion(
                runtime,
                task.marker,
            )
            assertion.require_answer_marker(launch, "dcode")
            results["answer_marker"] = TestResult(
                TestStatus.PASSED,
                f"dcode stdout answer contains {task.marker}",
            )

            trace_id = assertion.require_trace_id(
                launch,
                expected_count=1,
                selected_index=0,
            )
            test_context.report_progress(
                "runtime_stop",
                "stopping actraild and draining trace",
            )
            stopped = runtime.stop()
            if stopped is None or stopped.returncode != 0:
                returncode = None if stopped is None else stopped.returncode
                raise AssertionError(
                    f"actraild safe stop exited with {returncode}"
                )
            results["runtime_stop"] = TestResult(
                TestStatus.PASSED,
                "actraild drained trace finalization and stopped successfully",
            )

            test_context.report_progress(
                "trace_validation",
                "validating finalized LLM exchange",
            )
            request_count, response_count = (
                assertion.require_finalized_exchange(trace_id)
            )
            results["llm_exchange"] = TestResult(
                TestStatus.PASSED,
                f"trace-{trace_id} has {request_count} terminal paired request(s), "
                f"{response_count} response(s), and a linked marker exchange",
            )
            return TestResult(
                TestStatus.COMPOSITE,
                "dcode launch and LLM capture",
                results,
            )
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(
                TestStatus.COMPOSITE,
                "dcode launch and LLM capture",
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
