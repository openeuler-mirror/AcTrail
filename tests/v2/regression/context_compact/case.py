from __future__ import annotations

import json

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestCase, TestResult, TestStatus
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.runner import TestingContextSingleton

from .agents import CompactAgentCatalog
from .claude import ClaudeSessionDriver
from .config import ContextCompactConfig
from .driver import OpenCodeSessionDriver
from .scenario import SCENARIOS, AgentSpec


class ContextCompactCase(TestCase):
    def __init__(self, config: ContextCompactConfig):
        self._config = config

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        runtime: ActrailRuntime | None = None
        try:
            spec = self._select_spec(test_context)
            if spec is None:
                return TestResult(TestStatus.SKIPPED, "no available real agent in " + "/".join(self._config.agents))
            patch = PatchBuilder().with_values({
                "semantic_retention": {
                    "l0_llm_call": {
                        "context_compact": {
                            "enabled": self._config.mode != "disabled",
                        },
                    },
                },
            }).build().save(self._config.work_dir / "compact-mode.toml")
            runtime = ActrailRuntime.isolated(
                self._config.repo, self._config.bin_dir,
                self._config.command_timeout_seconds, test_context.output,
                self._config.work_dir,
                operator_config_patch=patch,
            )
            runtime.prepare()
            runs = []
            for scenario in SCENARIOS:
                try:
                    driver = ClaudeSessionDriver if spec.name == "claude" else OpenCodeSessionDriver
                    runs.append((scenario, driver(self._config, runtime, spec, scenario).run()))
                except Exception as error:
                    results[f"{spec.name}.{scenario.name}"] = TestResult(TestStatus.FAILED, str(error))
            stopped = runtime.stop()
            if stopped is None or stopped.returncode != 0:
                raise RuntimeError("actraild did not stop and drain successfully")
            for scenario, run in runs:
                try:
                    count = self._require_recorded_session(runtime, run.trace_id)
                    results[f"{spec.name}.{scenario.name}"] = TestResult(
                        TestStatus.PASSED, f"trace-{run.trace_id} agent.context_compaction={count}",
                    )
                except Exception as error:
                    results[f"{spec.name}.{scenario.name}"] = TestResult(TestStatus.FAILED, str(error))
        except Exception as error:
            results["actrail"] = TestResult(TestStatus.FAILED, str(error))
        finally:
            if runtime is not None:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode != 0:
                    results["runtime_stop"] = TestResult(TestStatus.FAILED, f"actraild stop exited with {stopped.returncode}")
        return TestResult(TestStatus.COMPOSITE, f"real-agent context compaction {self._config.mode}", results)

    def _require_recorded_session(self, runtime: ActrailRuntime, trace_id: int) -> int:
        result = runtime.run_checked(runtime.viewer_command(
            "--output-format", "json", "actions", "--trace-id", str(trace_id),
        ), echo=False)
        actions = json.loads(result.stdout)["actions"]
        requests = [action for action in actions if action["kind"] == "llm.request"]
        if not requests or not any(action["kind"] == "llm.response" for action in actions):
            raise AssertionError(f"trace-{trace_id} is missing LLM requests or responses")
        if not any(action["attributes"].get("llm.request.trajectory_id") for action in requests):
            raise AssertionError(f"trace-{trace_id} has no request trajectory")
        count = sum(action["kind"] == "agent.context_compaction" for action in actions)
        if self._config.mode == "disabled" and count != 0:
            raise AssertionError(f"trace-{trace_id} recorded context compaction while disabled")
        if self._config.mode == "inference" and count == 0:
            raise AssertionError(f"trace-{trace_id} has no agent.context_compaction actions")
        return count

    def _select_spec(self, context: TestingContextSingleton) -> AgentSpec | None:
        catalog = CompactAgentCatalog(self._config)
        for name in self._config.agents:
            spec = catalog.resolve(name)
            if spec is None:
                context.output.line(f"{name} executable not found; trying next candidate")
                continue
            context.output.line(f"checking {name} availability before starting capture")
            if context.check_agent_availability(name, spec.binary, spec.environment):
                context.output.line(f"selected context compaction agent: {name}")
                return spec
        return None
