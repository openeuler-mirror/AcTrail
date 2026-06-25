"""OTEL edge validation for docs example 07."""

from __future__ import annotations

import json
import shlex
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class SpanRecord:
    trace_id: str
    name: str
    attributes: dict[str, str]


@dataclass(frozen=True)
class EdgeValidation:
    facts: list[str]
    missing: list[str]


BASH_MARKER = "ACTRAIL_AGENT_TREE_OK"


def validate_agent_invocation_edge(
    path: Path,
    expected_xiaoo: str,
    expected_claude: str,
    expected_prompt: str,
    claude_extra_args: str,
) -> EdgeValidation:
    spans = read_otel_spans(path)
    expected_child_command = " ".join(
        [Path(expected_claude).name, *shlex.split(claude_extra_args), "-p", expected_prompt]
    )
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
    child_bash_command = find_child_bash_command_span(spans, edge.trace_id, child_pid)
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
    if child_bash_command is None:
        missing.append("matching Claude child Bash command.invocation with marker")
    else:
        facts.extend(
            [
                f"child Bash command action_id={child_bash_command.attributes.get('actrail.action.id')}",
                f"child Bash command pid={child_bash_command.attributes.get('process.pid')}",
                f"child Bash command executable={child_bash_command.attributes.get('process.executable')}",
                f"child Bash command line={child_bash_command.attributes.get('command.line')}",
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


def find_child_bash_command_span(
    spans: list[SpanRecord],
    trace_id: str,
    pid: str,
) -> SpanRecord | None:
    for span in spans:
        attributes = span.attributes
        command_line = attributes.get("command.line", "")
        executable = executable_basename(
            attributes.get("process.executable", "") or attributes.get("executable", "")
        )
        if span.trace_id != trace_id:
            continue
        if attributes.get("actrail.action.kind") != "command.invocation":
            continue
        if attributes.get("actrail.action.status") != "success":
            continue
        if attributes.get("actrail.action.completeness") != "complete":
            continue
        if attributes.get("process.parent.pid") != pid:
            continue
        if BASH_MARKER not in command_line or "printf" not in command_line:
            continue
        if executable in {"bash", "sh"} or "bash -c" in command_line or "sh -c" in command_line:
            return span
    return None


def executable_basename(value: str) -> str:
    return Path(value).name if value else ""


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
