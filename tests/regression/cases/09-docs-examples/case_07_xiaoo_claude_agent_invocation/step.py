"""Docs example 07 xiaoO-to-Claude regression step."""

from __future__ import annotations

import json
from dataclasses import dataclass
from pathlib import Path

from model import FAIL, PASS, SKIP, CaseResult
from workload_config import read_config, required

from helpers import (
    add_expected_found_check,
    bullet_evidence,
    evidence_detail,
    fail_step,
    line_evidence,
)


def run_agent_invocation(env, result: CaseResult, workload: dict[str, str]) -> str:
    name = "docs 07 xiaoO invokes Claude"
    docs_workload = read_config(
        env.repo_root / "docs/examples/07.xiaoo-claude-agent-invocation/workload.conf"
    )
    xiaoo = env.resolve_executable_reference(required(docs_workload, "agent_command"))
    claude = env.which("claude")
    if xiaoo is None or claude is None:
        result.add_check(
            name,
            SKIP,
            evidence_detail("xiaoo and claude on PATH", f"xiaoo={xiaoo}; claude={claude}"),
            "docs example 07 requires both real agent CLIs",
        )
        return SKIP
    marker = required(docs_workload, "expected_output_fragment")
    claude_check = env.run(
        [claude, "-p", required(docs_workload, "direct_claude_prompt")],
        timeout=float(required(docs_workload, "claude_timeout_seconds")),
    )
    if claude_check.returncode != 0 or marker not in claude_check.output:
        result.command = claude_check.command
        result.stdout_tail = env.output_tail(claude_check.stdout)
        result.stderr_tail = env.output_tail(claude_check.stderr)
        result.add_check(
            name,
            SKIP,
            evidence_detail(
                f"direct claude output contains {marker}",
                f"exit={claude_check.returncode}; found={marker in claude_check.output}",
            ),
            "direct Claude Code availability failed before agent invocation",
        )
        return SKIP
    xiaoo_check = env.run(
        [
            str(xiaoo),
            "run",
            "--no-tools",
            "--max-turns",
            required(workload, "agent_invocation_xiaoo_availability_max_turns"),
            "--prompt",
            required(docs_workload, "direct_claude_prompt"),
        ],
        timeout=float(required(docs_workload, "launch_timeout_seconds")),
    )
    if xiaoo_check.returncode != 0 or marker not in xiaoo_check.output:
        result.command = xiaoo_check.command
        result.stdout_tail = env.output_tail(xiaoo_check.stdout)
        result.stderr_tail = env.output_tail(xiaoo_check.stderr)
        result.add_check(
            name,
            SKIP,
            evidence_detail(
                f"direct xiaoO output contains {marker}",
                f"exit={xiaoo_check.returncode}; found={marker in xiaoo_check.output}",
            ),
            "direct xiaoO availability failed before agent invocation",
        )
        return SKIP
    command = [
        env.python,
        str(env.repo_root / "docs/examples/07.xiaoo-claude-agent-invocation/run_e2e.py"),
        "--bin-dir",
        str(env.bin_dir),
    ]
    completed = env.run(command, timeout=float(required(workload, "agent_invocation_total_timeout_seconds")))
    result.command = completed.command
    result.stdout_tail = env.output_tail(completed.stdout)
    result.stderr_tail = env.output_tail(completed.stderr)
    result.report_paths.append(required(docs_workload, "otel_output_path"))
    if completed.returncode != 0:
        return fail_step(env, result, name, RuntimeError(f"exit={completed.returncode}"))
    expected = (marker, "agent_command_trace_id=", "otel_output=", "agent command e2e complete")
    status = PASS if all(fragment in completed.output for fragment in expected) else FAIL
    result.add_check(
        f"{name} script completion markers",
        status,
        evidence_detail(
            f"output contains {list(expected)}",
            script_completion_evidence(completed.output, expected),
        )
        if status == FAIL
        else evidence_detail(
            "docs e2e script reports completion",
            script_completion_evidence(completed.output, expected),
        ),
        "script markers only prove the docs runner completed; OTEL edge checks prove xiaoO invoked Claude",
    )
    if status == FAIL:
        return FAIL
    otel_path = Path(required(docs_workload, "otel_output_path"))
    edge = validate_agent_invocation_edge(
        otel_path,
        str(xiaoo),
        claude,
        required(docs_workload, "direct_claude_prompt"),
    )
    otel_status = PASS if not edge.missing else FAIL
    add_expected_found_check(
        result,
        f"{name} OTEL invocation edge",
        "one complete agent-labeled command.invocation whose child is Claude and whose parent is Claude's direct launcher",
        bullet_evidence(edge.facts + [f"missing: {item}" for item in edge.missing]),
        "docs example 07 must prove the direct launcher -> Claude semantic edge plus the xiaoO trace root",
        status=otel_status,
    )
    if otel_status == FAIL:
        return FAIL
    return status


def script_completion_evidence(output: str, fragments: tuple[str, ...]) -> str:
    return bullet_evidence([line_evidence(output, fragment) for fragment in fragments])


@dataclass(frozen=True)
class SpanRecord:
    trace_id: str
    name: str
    attributes: dict[str, str]


@dataclass(frozen=True)
class EdgeValidation:
    facts: list[str]
    missing: list[str]


def validate_agent_invocation_edge(
    path: Path,
    expected_xiaoo: str,
    expected_claude: str,
    expected_prompt: str,
) -> EdgeValidation:
    spans = read_otel_spans(path)
    expected_child_command = f"{Path(expected_claude).name} -p {expected_prompt}"
    edge = find_agent_command_span(spans, expected_claude, expected_child_command)
    if edge is None:
        return EdgeValidation(
            facts=[
                f"otel_output={path}",
                f"agent_command_spans={count_agent_command_spans(spans)}",
                f"command.invocation_spans={count_kind(spans, 'command.invocation')}",
                f"process.exec_spans={count_kind(spans, 'process.exec')}",
            ],
            missing=[
                "agent command.invocation span with expected child executable, child command, success, and complete",
            ],
        )
    parent_pid = edge.attributes["process.parent.pid"]
    child_pid = edge.attributes["agent.child.pid"]
    root_exec = find_process_exec_span_by_executable(spans, edge.trace_id, expected_xiaoo)
    parent_exec = find_process_exec_span(spans, edge.trace_id, parent_pid, None, None)
    child_exec = find_process_exec_span(
        spans,
        edge.trace_id,
        child_pid,
        expected_claude,
        expected_child_command,
    )
    child_llm = find_llm_request_span(spans, edge.trace_id, child_pid)
    facts = [
        f"otel_output={path}",
        f"agent command trace_id={edge.trace_id}",
        f"agent command action_id={edge.attributes.get('actrail.action.id')}",
        "agent command status="
        f"{edge.attributes.get('actrail.action.status')} "
        f"completeness={edge.attributes.get('actrail.action.completeness')}",
        f"agent command trigger={edge.attributes.get('agent.invocation.trigger')}",
        f"agent command evidence_action_id={edge.attributes.get('agent.invocation.evidence_action_id')}",
        f"agent command parent pid={parent_pid}",
        f"agent command child pid={child_pid} executable={edge.attributes.get('agent.child.executable')}",
        f"agent command child command_line={edge.attributes.get('agent.child.command_line')}",
    ]
    missing: list[str] = []
    if root_exec is None:
        missing.append("matching xiaoO process.exec span in the same trace")
    else:
        facts.extend(
            [
                f"root xiaoO process.exec action_id={root_exec.attributes.get('actrail.action.id')}",
                f"root xiaoO process.exec pid={root_exec.attributes.get('process.pid')} executable={root_exec.attributes.get('executable')}",
                f"root xiaoO process.exec command_line={root_exec.attributes.get('command_line')}",
            ]
        )
    if parent_exec is None:
        missing.append("matching direct parent process.exec span with same trace_id and parent pid")
    else:
        facts.extend(
            [
                f"parent process.exec action_id={parent_exec.attributes.get('actrail.action.id')}",
                f"parent process.exec pid={parent_exec.attributes.get('process.pid')} executable={parent_exec.attributes.get('executable')}",
                f"parent process.exec command_line={parent_exec.attributes.get('command_line')}",
            ]
        )
    if child_exec is None:
        missing.append("matching Claude process.exec span with same trace_id, child pid, and child command")
    else:
        facts.extend(
            [
                f"child process.exec action_id={child_exec.attributes.get('actrail.action.id')}",
                f"child process.exec pid={child_exec.attributes.get('process.pid')} executable={child_exec.attributes.get('executable')}",
                f"child process.exec command_line={child_exec.attributes.get('command_line')}",
            ]
        )
    if child_llm is None:
        missing.append("matching Claude llm.request span with same trace_id and child pid")
    else:
        facts.extend(
            [
                f"child llm.request action_id={child_llm.attributes.get('actrail.action.id')}",
                f"child llm.request pid={child_llm.attributes.get('process.pid')}",
            ]
        )
    if edge.attributes.get("agent.invocation.trigger") != "child_llm_request":
        missing.append("agent.invocation.trigger=child_llm_request")
    if (
        child_llm is not None
        and edge.attributes.get("agent.invocation.evidence_action_id")
        != child_llm.attributes.get("actrail.action.id")
    ):
        missing.append("agent.invocation.evidence_action_id points to the child Claude llm.request action")
    if root_exec is not None and parent_exec is not None and child_exec is not None:
        facts.append(
            "pid linkage verified: "
            f"agent command process.parent.pid == parent process.pid == {parent_pid}; "
            f"agent command child pid == child process.pid == {child_pid}"
        )
        facts.append(
            "trace linkage verified: agent command, xiaoO exec, parent exec, and child exec "
            "share the same trace_id"
        )
    return EdgeValidation(facts, missing)


def read_otel_spans(path: Path) -> list[SpanRecord]:
    document = json.loads(path.read_text(encoding="utf-8"))
    return [
        SpanRecord(
            trace_id=str(span.get("traceId", "")),
            name=str(span.get("name", "")),
            attributes=otel_attributes(span),
        )
        for span in iter_otel_spans(document)
    ]


def find_agent_command_span(
    spans: list[SpanRecord],
    expected_claude: str,
    expected_child_command: str,
) -> SpanRecord | None:
    for span in spans:
        attributes = span.attributes
        if (
            attributes.get("actrail.action.kind") == "command.invocation"
            and attributes.get("invocation.kind") == "agent"
            and attributes.get("actrail.action.status") == "success"
            and attributes.get("actrail.action.completeness") == "complete"
            and attributes.get("agent.child.executable") == expected_claude
            and (
                attributes.get("agent.child.command_line") == expected_child_command
                or attributes.get("command.line") == expected_child_command
            )
            and attributes.get("process.parent.pid")
            and attributes.get("agent.child.pid")
        ):
            return span
    return None


def find_process_exec_span_by_executable(
    spans: list[SpanRecord],
    trace_id: str,
    executable: str,
) -> SpanRecord | None:
    for span in spans:
        attributes = span.attributes
        if (
            span.trace_id == trace_id
            and attributes.get("actrail.action.kind") == "process.exec"
            and attributes.get("executable") == executable
        ):
            return span
    return None


def find_process_exec_span(
    spans: list[SpanRecord],
    trace_id: str,
    pid: str,
    executable: str | None,
    command_line: str | None,
) -> SpanRecord | None:
    for span in spans:
        attributes = span.attributes
        if (
            span.trace_id == trace_id
            and attributes.get("actrail.action.kind") == "process.exec"
            and attributes.get("process.pid") == pid
            and (executable is None or attributes.get("executable") == executable)
            and (command_line is None or attributes.get("command_line") == command_line)
        ):
            return span
    return None


def find_llm_request_span(
    spans: list[SpanRecord],
    trace_id: str,
    pid: str,
) -> SpanRecord | None:
    for span in spans:
        attributes = span.attributes
        if (
            span.trace_id == trace_id
            and attributes.get("actrail.action.kind") == "llm.request"
            and attributes.get("process.pid") == pid
        ):
            return span
    return None


def count_kind(spans: list[SpanRecord], kind: str) -> int:
    return sum(1 for span in spans if span.attributes.get("actrail.action.kind") == kind)


def count_agent_command_spans(spans: list[SpanRecord]) -> int:
    return sum(
        1
        for span in spans
        if span.attributes.get("actrail.action.kind") == "command.invocation"
        and span.attributes.get("invocation.kind") == "agent"
    )


def iter_otel_spans(document: dict):
    for resource_span in document.get("resourceSpans", []):
        for scope_span in resource_span.get("scopeSpans", []):
            yield from scope_span.get("spans", [])


def otel_attributes(span: dict) -> dict[str, str]:
    values: dict[str, str] = {}
    for attribute in span.get("attributes", []):
        key = attribute.get("key")
        value = attribute.get("value", {})
        if key and value:
            values[key] = str(next(iter(value.values())))
    return values
