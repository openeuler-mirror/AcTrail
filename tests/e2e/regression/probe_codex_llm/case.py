"""Run Codex with scoped lifecycle management and verify finalized LLM capture."""
from __future__ import annotations

from tests.e2e.common.actrail_runtime import ActrailRuntime, LaunchOptions
from tests.e2e.common.core import TestRunDirectories, AgentBinaryNotFoundError, TestCase, TestResult
from tests.e2e.common.runner import TestingContextSingleton
from tests.e2e.common.llm_trace_assertion import LLMTraceAssertion
from .config import ProbeCodexLLMConfig
from .constants import CASE_CONSTANTS
from .task import ProbeCodexLLMTask


class ProbeCodexLLMCase(TestCase):
    case_id = 'probe_codex_llm'
    description = 'Run Codex through actrailctl launch and verify LLM capture'


    def __init__(self, directories: TestRunDirectories) -> None:
        config = ProbeCodexLLMConfig.from_environment(directories)
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        context = TestingContextSingleton.current()
        with context.case("Codex launch and LLM capture") as case:
            with case.step("Check Codex availability"):
                try:
                    task = ProbeCodexLLMTask(self._config)
                except AgentBinaryNotFoundError as error:
                    case.skip(str(error))
                if not context.check_agent_availability(
                    "codex", task.binary, task.environment()
                ):
                    case.skip("Codex external availability check failed")

            with case.step("Capture Codex activity"):
                with ActrailRuntime.isolated(
                    self._config.repo, self._config.bin_dir,
                    self._config.command_timeout_seconds, context.output,
                    self._config.work_dir,
                    agent_invocation_commands=[task.native_binary],
                    launch_options=LaunchOptions(host_ebpf="auto"),
                ) as runtime:
                    with case.step("Start AcTrail"):
                        runtime.start_actraild()
                    with case.step("Send a request with Codex"):
                        with task.create_agent().launchWithAcTrailCtl(stream_mode=False) as agent:
                            agent.run(CASE_CONSTANTS.prompt)
                            agent.join_all(self._config.launch_timeout_seconds)
                            result = agent.results[-1]
                            if not result.succeeded:
                                raise AssertionError(result.error or "Codex task failed")
                            launch = agent.launch_result
                        if agent.returncode != 0:
                            raise AssertionError(f"Codex launch exited with {agent.returncode}")

            with case.step("Check capture permissions"):
                permissions = launch.permissions
                if permissions is None:
                    raise AssertionError("Launch did not report deployment permissions")
                if permissions.requested != "host_ebpf:auto,seccomp_notify:auto":
                    raise AssertionError(f"Unexpected requested permissions: {permissions.requested}")
                if (permissions.selected != "host_ebpf:enabled,seccomp_notify:disabled"
                        or permissions.degraded):
                    raise AssertionError(f"Unexpected selected permissions: {permissions}")

            # Runtime 退出完成停机和排空后，检查最终请求响应。
            with case.step("Check the recorded LLM exchange"):
                LLMTraceAssertion(runtime).require_finalized_exchange(
                    launch.trace_id,
                    reply_text=task.read_reply(),
                )
        return case.result
