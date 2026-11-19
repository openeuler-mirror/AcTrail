from __future__ import annotations

import secrets
from collections import Counter
from pathlib import Path
from typing import Any

from tests.e2e.common.actrail_runtime import CommandResult, LaunchOptions, ToolAccess
from tests.e2e.common.agents import Agent
from tests.e2e.common.runner import TestingContextSingleton

from .environment import SemanticActionBoundariesEnvironment
from .observation import ExportedSpan, SemanticActionObservation

from .constants import CASE_CONSTANTS


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
        agent: Agent,
    ):
        self._environment = environment
        self._agent = agent
        self._observation = SemanticActionObservation(environment)

    def run(self, case) -> None:
        with case.step("Check which actions are exported at completion") as step:
            counts = self._run_boundary_round()
            step.message = ", ".join(f"{kind}={count}" for kind, count in sorted(counts.items()))
        with case.step("Check process execution edge cases"):
            self._run_exec_edge_cases(case)
        with case.step("Check observation runtime health"):
            self._observation.require_runtime_healthy()

    def _run_boundary_round(self) -> Counter[str]:
        self._environment.enable_observed_kinds(
            set(self._OBSERVED_KINDS)
        )
        marker = (
            "SEMANTIC_ACTION_BOUNDARIES_"
            f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
        )
        TestingContextSingleton.current().report_progress(
            "Run the Agent to observe action completion",
            f"launching {self._agent.kind} boundary trace",
        )
        trace_id, _ = self._launch_agent(marker)
        TestingContextSingleton.current().report_progress(
            "Wait for completed actions to be exported",
            "waiting for finalized exported actions",
        )
        self._observation.wait_for_terminal_trace(trace_id)
        self._observation.wait_for_exported_kinds(
            marker,
            set(self._OBSERVED_KINDS),
        )
        spans = self._observation.extract_exported_spans(marker)
        process_id = self._agent_process_id(spans)
        root_spans = [
            span for span in spans if span.process_id == process_id
        ]
        counts = Counter(span.kind for span in root_spans)
        expected_exact = {
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
            {"llm.request", "process.exec", "command.invocation"},
        )
        missing = {"llm.request", "process.exec", "command.invocation"}.difference(counts)
        if missing or mismatches or unexpected:
            raise AssertionError(
                f"semantic action root process {process_id} counts are "
                f"{dict(sorted(counts.items()))}, expected exact "
                f"{dict(sorted(expected_exact.items()))}, "
                "at least one LLM request, exec and command, and no other kinds"
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

    def _run_exec_edge_cases(self, case) -> None:
        with case.step("Check a failed exec attempt"):
            self._require_failed_exec_attempt()
        with case.step("Check exec completion captured by eBPF"):
            self._require_ebpf_only_exec_completion()
        with case.step("Check a nonzero process exit"):
            self._require_nonzero_process_exit()

    def _require_failed_exec_attempt(self) -> None:
        target = self._environment.config.work_dir / "non-executable"
        target.write_text("#!/bin/sh\nexit 0\n", encoding="utf-8")
        target.chmod(0o644)
        marker = (
            "SEMANTIC_ACTION_SECCOMP_ONLY_"
            f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
        )
        try:
            launch = self._launch_command(
                marker,
                [
                    "/bin/bash",
                    "-c",
                    'printf "%s\\n" "$1"; exec "$2"',
                    "actrail-boundary",
                    marker,
                    str(target),
                ],
            )
            if launch.returncode == 0:
                raise AssertionError(
                    "non-executable target unexpectedly succeeded"
                )
            trace_id = self._observation.require_trace_id(launch)
            self._observation.wait_for_terminal_trace(trace_id)
            spans = self._observation.wait_for_marker_spans(
                marker,
                {"process.exec", "process.exit"},
            )
            failed_execs = [
                span
                for span in spans
                if span.kind == "process.exec"
                and span.executable == str(target)
            ]
            if len(failed_execs) != 1:
                raise AssertionError(
                    "failed target exec count is "
                    f"{len(failed_execs)}, expected 1"
                )
            failed_exec = failed_execs[0]
            if failed_exec.status_code != "STATUS_CODE_ERROR":
                raise AssertionError(
                    "failed target exec has status "
                    f"{failed_exec.status_code!r}"
                )
            action = self._observation.stored_action(
                trace_id,
                failed_exec.action_id,
            )
            attributes = action.get("attributes")
            if not isinstance(attributes, dict):
                raise AssertionError(
                    "failed target exec has no attributes"
                )
            try:
                result = int(str(attributes.get("result")))
            except (TypeError, ValueError) as error:
                raise AssertionError(
                    "failed target exec has no numeric result"
                ) from error
            if (
                attributes.get("exec.path") != str(target)
                or attributes.get("exec.result") != "failed"
                or result >= 0
            ):
                raise AssertionError(
                    "failed target exec attributes are inconsistent: "
                    f"path={attributes.get('exec.path')!r}, "
                    f"exec.result={attributes.get('exec.result')!r}, "
                    f"result={result}"
                )
            evidence = action.get("evidence")
            roles = (
                {
                    item.get("role")
                    for item in evidence
                    if isinstance(item, dict)
                }
                if isinstance(evidence, list)
                else set()
            )
            if (
                "process.exec.completed" not in roles
                or roles.difference(
                    {
                        "process.exec.intent",
                        "process.exec.completed",
                    }
                )
            ):
                raise AssertionError(
                    "failed target exec evidence roles are "
                    f"{sorted(str(role) for role in roles)}"
                )
        finally:
            target.unlink(missing_ok=True)

    def _require_ebpf_only_exec_completion(self) -> None:
        marker = (
            "SEMANTIC_ACTION_EBPF_ONLY_"
            f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
        )
        launch = self._launch_command(
            marker,
            [
                "/bin/bash",
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
                f"{launch.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}"
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
            f"{secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)}"
        )
        launch = self._launch_command(
            marker,
            [
                "/bin/bash",
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

    def _launch_agent(self, marker: str):
        prompt = "Briefly explain how a process executes a program."
        options = LaunchOptions(
            name=marker, host_ebpf="auto", tool_access=ToolAccess.NONE,
            max_turns=self._environment.config.agent_max_turns,
        )
        with self._environment.runtime.launch_scope(options):
            with self._agent.launchWithAcTrailCtl(stream_mode=False) as agent:
                agent.run(prompt)
                agent.join_all(self._environment.config.launch_timeout_seconds)
                result = agent.results[-1]
                if not result.succeeded:
                    raise AssertionError(result.error or "Agent task failed")
                trace_id = agent.launch_result.trace_id
            if agent.returncode != 0:
                raise AssertionError(f"Agent exited with {agent.returncode}")
        return trace_id, result

    def _launch_command(
        self,
        marker: str,
        command: list[str],
        *,
        host_ebpf: str = CASE_CONSTANTS.launch_command_host_ebpf,
        seccomp_notify: str = CASE_CONSTANTS.launch_command_seccomp_notify,
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

    def _agent_process_id(self, spans: list[ExportedSpan]) -> str:
        processes = {span.process_id for span in spans if span.kind == "agent.identity"}
        if len(processes) != 1:
            raise AssertionError(f"expected one actual Agent process, found {sorted(processes)}")
        return next(iter(processes))

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
            if action.get("kind") == "process.exec":
                self._require_completed_exec(action)
            elif action.get("kind") == "llm.request":
                self._require_llm_request_boundary(action)

    @staticmethod
    def _require_llm_request_boundary(action: dict[str, Any]) -> None:
        attributes = action.get("attributes")
        if not isinstance(attributes, dict):
            raise AssertionError(
                f"llm.request {action.get('action_id')} has no attributes"
            )
        method = str(attributes.get("http.request.method") or "").upper()
        path = attributes.get("url.path")
        if path == "/v1" and method != "POST":
            raise AssertionError(
                f"llm.request {action.get('action_id')} classified the "
                f"ordinary {method or 'unknown-method'} /v1 probe"
            )

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
        completed_only = {"process.exec.completed"}
        completed_with_synchronous_intent = {
            "process.exec.intent",
            "process.exec.completed",
        }
        if roles not in {
            frozenset(completed_only),
            frozenset(completed_with_synchronous_intent),
        }:
            raise AssertionError(
                f"process.exec {action.get('action_id')} evidence roles "
                f"are {sorted(str(role) for role in roles)}, expected "
                f"{sorted(completed_only)} or "
                f"{sorted(completed_with_synchronous_intent)}"
            )
