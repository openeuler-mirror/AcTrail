"""Run QoderCli with scoped lifecycle management and verify finalized LLM capture."""
from __future__ import annotations

from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.core import TestRunDirectories, AgentBinaryNotFoundError, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.llm_trace_assertion import LLMTraceAssertion
from .config import ProbeQoderCliLLMConfig
from .constants import CASE_CONSTANTS
from .task import ProbeQoderCliLLMTask


class ProbeQoderCliLLMCase(TestCase):
    case_id = 'probe_qodercli_llm'
    description = 'Run qodercli through actrailctl launch and verify LLM capture'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ProbeQoderCliLLMConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("QoderCli launch and LLM capture") as case:
            with case.step("Check Qoder CLI availability"):
                try:
                    task = ProbeQoderCliLLMTask(self._config)
                except AgentBinaryNotFoundError as error:
                    case.skip(str(error))
                if not context.check_agent_availability(
                    "qodercli", task.binary, task.environment()
                ):
                    case.skip("QoderCli external availability check failed")

            with case.step("Capture Qoder CLI activity"):
                with ActrailRuntime.isolated(
                    self._config.repo, self._config.bin_dir,
                    self._config.command_timeout_seconds, context.output,
                    self._config.work_dir,
                ) as runtime:
                    with case.step("Start AcTrail"):
                        runtime.start_actraild()
                    with case.step("Load the Qoder LLM codec"):
                        runtime.run_checked(runtime.daemon_command(
                            "plugin", "load", "--manifest",
                            self._config.repo / "examples/plugins/wasm-legacy/llm-codec-qoder/plugin.toml",
                            "--instance", "qoder.llm-codec",
                        ))
                    with case.step("Send a request with Qoder CLI"):
                        with task.create_agent().launchWithAcTrailCtl(stream_mode=True) as agent:
                            agent.run(CASE_CONSTANTS.prompt)
                            agent.join_all(self._config.launch_timeout_seconds)
                            result = agent.results[-1]
                            if not result.succeeded:
                                raise AssertionError(result.error or "QoderCli task failed")
                            launch = agent.launch_result
                        if agent.returncode != 0:
                            raise AssertionError(f"QoderCli launch exited with {agent.returncode}")

            # Runtime 退出完成停机和排空后，检查最终请求响应。
            with case.step("Check the recorded LLM exchange"):
                LLMTraceAssertion(runtime).require_finalized_exchange(
                    launch.trace_id,
                    reply_text=result.output,
                    request_protocol_id=CASE_CONSTANTS.request_protocol_id,
                )
        return case.result
