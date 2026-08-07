from __future__ import annotations

import secrets
from collections import Counter
from pathlib import Path
from typing import Any

from tests.v2.common.actrail_runtime import CommandResult
from tests.v2.common.agent_selection import AgentSelection
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .environment import SemanticActionBoundariesEnvironment
from .observation import ExportedSpan, SemanticActionObservation


class SemanticActionBoundariesTask:
    _OBSERVED_KINDS = frozenset(
        {
            "process.exec",
            "process.exit",
            "agent.identity",
            "agent.exit",
            "llm.request",
            "command.invocation",
        }
    )
    _EXPORT_ONLY_KINDS = frozenset({"process.exit", "agent.exit"})
    _PERSISTED_KINDS = _OBSERVED_KINDS.difference(_EXPORT_ONLY_KINDS)

    def __init__(
        self,
        environment: SemanticActionBoundariesEnvironment,
        agent: AgentSelection,
        test_context: TestingContextSingleton,
    ):
        self._environment = environment
        self._agent = agent
        self._test_context = test_context
        self._observation = SemanticActionObservation(environment)

    def run(self) -> dict[str, TestResult]:
        results: dict[str, TestResult] = {}
        self._test_context.report_progress(
            "boundary_round",
            "testing agent action boundaries",
        )
        boundary_counts = self._run_boundary_round()
        results["terminal-actions"] = TestResult(
            TestStatus.PASSED,
            "root process one-shot counts: "
            + ", ".join(
                f"{kind}={count}"
                for kind, count in sorted(boundary_counts.items())
            ),
        )
        self._run_exec_edge_cases()
        results["exec-edge-cases"] = TestResult(
            TestStatus.PASSED,
            "seccomp-only failure, eBPF-only completion, and nonzero "
            "exit passed",
        )
        self._test_context.report_progress(
            "runtime_health",
            "checking observation runtime health",
        )
        self._observation.require_runtime_healthy()
        results["observation_runtime"] = TestResult(
            TestStatus.PASSED,
            "semantic action observation remains active with "
            "dropped_records=0",
        )
        return results

    def _run_boundary_round(self) -> Counter[str]:
        self._environment.enable_observed_kinds(
            set(self._OBSERVED_KINDS)
        )
        marker = (
            "SEMANTIC_ACTION_BOUNDARIES_"
            f"{secrets.token_hex(6)}"
        )
        self._test_context.report_progress(
            "boundary_launch",
            f"launching {self._agent.kind} boundary trace",
        )
        launch = self._launch_agent(marker)
        if launch.returncode != 0:
            raise AssertionError(
                "semantic action boundaries: actrailctl launch exited "
                f"with {launch.returncode}\n{launch.output[-4000:]}"
            )
        if marker not in launch.output:
            raise AssertionError(
                f"semantic action boundaries: {self._agent.kind} output "
                f"does not contain marker {marker}"
            )
        self._test_context.report_progress(
            "boundary_observe",
            "waiting for finalized exported actions",
        )
        trace_id = self._observation.require_trace_id(launch)
        self._observation.wait_for_terminal_trace(trace_id)
        self._observation.wait_for_exported_kinds(
            marker,
            set(self._OBSERVED_KINDS),
        )
        spans = self._observation.extract_exported_spans(marker)
        process_id = self._root_process_id(spans)
        root_spans = [
            span for span in spans if span.process_id == process_id
        ]
        counts = Counter(span.kind for span in root_spans)
        expected_exact = {
            "process.exec": 2,
            "command.invocation": 2,
            "agent.identity": 1,
            "process.exit": 1,
            "agent.exit": 1,
        }
        mismatches = {
            kind: (counts.get(kind, 0), expected_count)
            for kind, expected_count in expected_exact.items()
            if counts.get(kind, 0) != expected_count
        }
        unexpected = set(counts).difference(
            expected_exact,
            {"llm.request"},
        )
        if counts.get("llm.request", 0) < 2 or mismatches or unexpected:
            raise AssertionError(
                f"semantic action root process {process_id} counts are "
                f"{dict(sorted(counts.items()))}, expected exact "
                f"{dict(sorted(expected_exact.items()))}, "
                "llm.request>=2, and no other kinds"
            )
        action_id_counts = Counter(
            span.action_id for span in root_spans
        )
        duplicates = {
            action_id: count
            for action_id, count in action_id_counts.items()
            if not action_id or count != 1
        }
        if duplicates:
            raise AssertionError(
                "semantic action root process has non-one-shot action "
                f"IDs: {dict(sorted(duplicates.items()))}"
            )
        self._require_stored_boundary_actions(
            trace_id,
            int(process_id),
            root_spans,
        )
        return counts

    def _run_exec_edge_cases(self) -> None:
        self._test_context.report_progress(
            "seccomp_exec",
            "testing failed seccomp exec",
        )
        self._require_seccomp_only_exec_attempt()
        self._test_context.report_progress(
            "ebpf_exec",
            "testing eBPF-only exec completion",
        )
        self._require_ebpf_only_exec_completion()
        self._test_context.report_progress(
            "nonzero_exit",
            "testing nonzero process exit",
        )
        self._require_nonzero_process_exit()

    def _require_seccomp_only_exec_attempt(self) -> None:
        target = self._environment.config.work_dir / "non-executable"
        target.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        target.chmod(0o644)
        marker = (
            "SEMANTIC_ACTION_SECCOMP_ONLY_"
            f"{secrets.token_hex(6)}"
        )
        try:
            launch = self._launch_command(
                marker,
                [
                    "bash",
                    "-c",
                    'printf "%s\\n" "$1"; exec "$2"',
                    "actrail-boundary",
                    marker,
                    str(target),
                ],
            )
            if launch.returncode == 0:
                raise AssertionError(
                    "seccomp-only exec unexpectedly succeeded"
                )
            trace_id = self._observation.require_trace_id(launch)
            self._observation.wait_for_terminal_trace(trace_id)
            spans = self._observation.wait_for_marker_spans(
                marker,
                {"process.exec", "process.exit"},
            )
            if any(
                span.executable == str(target)
                for span in spans
            ):
                raise AssertionError(
                    "failed seccomp-only exec produced a semantic action"
                )
        finally:
            target.unlink(missing_ok=True)

    def _require_ebpf_only_exec_completion(self) -> None:
        marker = (
            "SEMANTIC_ACTION_EBPF_ONLY_"
            f"{secrets.token_hex(6)}"
        )
        launch = self._launch_command(
            marker,
            [
                "bash",
                "-c",
                'printf "%s\\n" "$1"; exec /bin/true',
                "actrail-boundary",
                marker,
            ],
            host_ebpf="required",
            seccomp_notify="disabled",
        )
        if launch.returncode != 0:
            raise AssertionError(
                f"eBPF-only exec exited with {launch.returncode}\n"
                f"{launch.output[-4000:]}"
            )
        trace_id = self._observation.require_trace_id(launch)
        self._observation.wait_for_terminal_trace(trace_id)
        spans = self._observation.wait_for_marker_spans(
            marker,
            {"process.exec", "process.exit"},
        )
        true_execs = [
            span
            for span in spans
            if span.kind == "process.exec"
            and Path(span.executable).name == "true"
        ]
        if len(true_execs) != 1:
            raise AssertionError(
                f"eBPF-only /bin/true exec count is {len(true_execs)}, "
                "expected 1"
            )
        action = self._observation.stored_action(
            trace_id,
            true_execs[0].action_id,
        )
        evidence = action.get("evidence")
        if not isinstance(evidence, list):
            raise AssertionError(
                "eBPF-only process.exec has no evidence array"
            )
        roles = {
            item.get("role")
            for item in evidence
            if isinstance(item, dict)
        }
        if roles != {"process.exec.completed"}:
            raise AssertionError(
                "eBPF-only process.exec evidence roles are "
                f"{sorted(str(role) for role in roles)}"
            )

    def _require_nonzero_process_exit(self) -> None:
        marker = (
            "SEMANTIC_ACTION_NONZERO_EXIT_"
            f"{secrets.token_hex(6)}"
        )
        launch = self._launch_command(
            marker,
            [
                "bash",
                "-c",
                'printf "%s\\n" "$1"; exit 17',
                "actrail-boundary",
                marker,
            ],
        )
        if launch.returncode != 17:
            raise AssertionError(
                f"nonzero exit returned {launch.returncode}, expected 17"
            )
        trace_id = self._observation.require_trace_id(launch)
        self._observation.wait_for_terminal_trace(trace_id)
        spans = self._observation.wait_for_marker_spans(
            marker,
            {"process.exec", "process.exit"},
        )
        bash_execs = [
            span
            for span in spans
            if span.kind == "process.exec"
            and Path(span.executable).name == "bash"
        ]
        if len(bash_execs) != 1:
            raise AssertionError(
                f"nonzero trace bash exec count is {len(bash_execs)}, "
                "expected 1"
            )
        process_exits = [
            span
            for span in spans
            if span.kind == "process.exit"
            and span.process_id == bash_execs[0].process_id
        ]
        if len(process_exits) != 1:
            raise AssertionError(
                f"nonzero process exit count is {len(process_exits)}, "
                "expected 1"
            )
        process_exit = process_exits[0]
        if (
            process_exit.exit_code != "17"
            or process_exit.status_code != "STATUS_CODE_ERROR"
        ):
            raise AssertionError(
                "nonzero process exit has "
                f"exit_code={process_exit.exit_code!r}, "
                f"status={process_exit.status_code!r}"
            )
        self._observation.stored_action(
            trace_id,
            bash_execs[0].action_id,
        )

    def _launch_agent(self, marker: str) -> CommandResult:
        prompt = (
            f'Reply with exactly "{marker}" and nothing else. Do not use tools.'
        )
        agent_command = self._agent.command(prompt)
        return self._environment.runtime.run(
            [
                self._environment.runtime.actrailctl,
                "--config",
                self._environment.config.operator_config,
                "launch",
                "--name",
                marker,
                "--",
                "bash",
                "-lc",
                'cat /etc/hostname >/dev/null; exec "$@"',
                "actrail-semantic-boundaries",
                *agent_command,
            ],
            timeout_seconds=self._environment.config.launch_timeout_seconds,
            environment=self._agent.environment,
        )

    def _launch_command(
        self,
        marker: str,
        command: list[str],
        *,
        host_ebpf: str = "auto",
        seccomp_notify: str = "auto",
    ) -> CommandResult:
        return self._environment.runtime.run(
            [
                self._environment.runtime.actrailctl,
                "--config",
                self._environment.config.operator_config,
                "launch",
                "--name",
                marker,
                "--host-ebpf",
                host_ebpf,
                "--seccomp-notify",
                seccomp_notify,
                "--",
                *command,
            ],
            timeout_seconds=self._environment.config.launch_timeout_seconds,
        )

    def _root_process_id(
        self,
        spans: list[ExportedSpan],
    ) -> str:
        execs_by_process: dict[str, list[ExportedSpan]] = {}
        for span in spans:
            if span.kind == "process.exec":
                execs_by_process.setdefault(
                    span.process_id,
                    [],
                ).append(span)
        agent_name = self._agent.binary.name
        candidates = []
        for process_id, execs in execs_by_process.items():
            executable_names = {
                Path(span.executable).name
                for span in execs
            }
            if (
                len(execs) == 2
                and "bash" in executable_names
                and agent_name in executable_names
            ):
                candidates.append(process_id)
        if len(candidates) != 1:
            raise AssertionError(
                "semantic action boundaries expected one bash-to-agent "
                f"process, found {sorted(candidates)}"
            )
        return candidates[0]

    def _require_stored_boundary_actions(
        self,
        trace_id: int,
        process_id: int,
        exported_spans: list[ExportedSpan],
    ) -> None:
        document = self._observation.viewer_json(
            ["actions", "--trace-id", str(trace_id)]
        )
        actions = document.get("actions")
        if not isinstance(actions, list):
            raise AssertionError(
                "actrailviewer actions returned no actions array"
            )
        root_actions = [
            action
            for action in actions
            if isinstance(action, dict)
            and isinstance(action.get("process"), dict)
            and action["process"].get("process_id") == process_id
        ]
        stored_observed_actions = [
            action
            for action in root_actions
            if action.get("kind") in self._OBSERVED_KINDS
        ]
        stored_export_only = Counter(
            str(action.get("kind"))
            for action in stored_observed_actions
            if action.get("kind") in self._EXPORT_ONLY_KINDS
        )
        if stored_export_only:
            raise AssertionError(
                "semantic action storage contains export-only root "
                f"actions: {dict(sorted(stored_export_only.items()))}"
            )
        stored_persisted_actions = [
            action
            for action in stored_observed_actions
            if action.get("kind") in self._PERSISTED_KINDS
        ]
        exported_pairs = Counter(
            (span.action_id, span.kind)
            for span in exported_spans
            if span.kind in self._PERSISTED_KINDS
        )
        stored_pairs = Counter(
            (
                str(action.get("action_id") or ""),
                str(action.get("kind") or ""),
            )
            for action in stored_persisted_actions
        )
        if stored_pairs != exported_pairs:
            missing = exported_pairs - stored_pairs
            unexpected = stored_pairs - exported_pairs
            raise AssertionError(
                "semantic action stored root actions differ from online "
                f"export: missing={dict(sorted(missing.items()))}, "
                f"unexpected={dict(sorted(unexpected.items()))}"
            )
        for action in stored_persisted_actions:
            if action.get("kind") != "process.exec":
                continue
            self._require_completed_exec(action)

    @staticmethod
    def _require_completed_exec(action: dict[str, Any]) -> None:
        attributes = action.get("attributes")
        if not isinstance(attributes, dict):
            raise AssertionError(
                f"process.exec {action.get('action_id')} has no attributes"
            )
        executable = str(
            attributes.get("process.executable") or ""
        )
        intent_path = str(attributes.get("exec.path") or "")
        if (
            not executable
            or not intent_path
            or Path(executable).name != Path(intent_path).name
        ):
            raise AssertionError(
                f"process.exec {action.get('action_id')} paired "
                f"completion {executable!r} with intent {intent_path!r}"
            )
        evidence = action.get("evidence")
        if not isinstance(evidence, list):
            raise AssertionError(
                f"process.exec {action.get('action_id')} has no "
                "evidence array"
            )
        roles = {
            item.get("role")
            for item in evidence
            if isinstance(item, dict)
        }
        expected_roles = {
            "process.exec.intent",
            "process.exec.completed",
        }
        if roles != expected_roles:
            raise AssertionError(
                f"process.exec {action.get('action_id')} evidence roles "
                f"are {sorted(str(role) for role in roles)}, expected "
                f"{sorted(expected_roles)}"
            )
