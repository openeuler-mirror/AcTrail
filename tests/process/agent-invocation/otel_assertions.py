from __future__ import annotations

import json
from pathlib import Path


def require_claude_exec_span(document: dict) -> None:
    span = find_claude_exec_span(document)
    if span is None:
        raise RuntimeError("missing claude process.exec span")
    attrs = span_attrs(span)
    if attrs.get("seccomp_observed") != "true":
        raise RuntimeError("claude process.exec span was not marked seccomp_observed=true")
    command_line = attrs.get("command_line", "")
    if "claude" not in command_line or "-p" not in command_line:
        raise RuntimeError(f"claude command_line missing expected argv: {command_line}")


def require_claude_llm_request_span(document: dict) -> None:
    if find_claude_llm_request_span(document) is None:
        raise RuntimeError("missing claude llm.request span")


def require_claude_llm_tool_response_span(document: dict) -> None:
    if find_claude_llm_tool_response_span(document) is None:
        raise RuntimeError("missing claude llm.response span with parsed tool_calls_json")


def require_agent_command_span(document: dict) -> None:
    span = find_claude_agent_command_span(document)
    if span is None:
        raise RuntimeError("missing direct parent -> claude agent command span")
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        raise RuntimeError("missing claude process.exec span")
    llm_span = find_claude_llm_request_span(document)
    if llm_span is None:
        raise RuntimeError("missing claude llm.request span")
    attrs = span_attrs(span)
    exec_attrs = span_attrs(exec_span)
    llm_attrs = span_attrs(llm_span)
    exec_pid = exec_attrs.get("process.pid", "")
    parent_pid = attrs.get("process.parent.pid", "")
    child_pid = attrs.get("agent.child.pid", "")
    child = attrs.get("agent.child.command_line", "") or attrs.get("command.line", "")
    trigger = attrs.get("agent.invocation.trigger", "")
    evidence_id = attrs.get("agent.invocation.evidence_action_id", "")
    llm_action_id = llm_attrs.get("actrail.action.id", "")
    if child_pid != exec_pid:
        raise RuntimeError(f"agent command child pid {child_pid} does not match claude pid {exec_pid}")
    if llm_attrs.get("process.pid", "") != child_pid:
        raise RuntimeError("agent command evidence is not from the Claude child pid")
    if trigger != "child_llm_request":
        raise RuntimeError(f"agent command trigger is not child_llm_request: {trigger}")
    if not evidence_id or evidence_id != llm_action_id:
        raise RuntimeError(
            "agent command evidence_action_id does not point to Claude llm.request: "
            f"edge={evidence_id}; llm={llm_action_id}"
        )
    if not parent_pid or parent_pid == child_pid:
        raise RuntimeError(f"agent command parent pid is not a direct external launcher: {parent_pid}")
    if "claude" not in child:
        raise RuntimeError(f"agent command child is not claude: {child}")


def evidence_is_complete(document: dict) -> bool:
    return (
        find_claude_exec_span(document) is not None
        and find_claude_llm_request_span(document) is not None
        and find_claude_llm_tool_response_span(document) is not None
        and find_claude_agent_command_span(document) is not None
    )


def find_claude_exec_span(document: dict) -> dict | None:
    fallback: list[dict] = []
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "process.exec":
            continue
        if attrs.get("seccomp_observed") != "true":
            continue
        if not is_claude_prompt_exec(span, attrs):
            continue
        if is_claude_executable(span, attrs):
            return span
        fallback.append(span)
    for span in fallback:
        attrs = span_attrs(span)
        if has_llm_request_for_pid(document, attrs.get("process.pid", "")):
            return span
    if fallback:
        return fallback[0]
    return None


def is_claude_prompt_exec(span: dict, attrs: dict[str, str]) -> bool:
    command_line = attrs.get("command_line", "")
    argv = attrs.get("argv", "")
    if "-p" not in command_line and "\n-p\n" not in argv:
        return False
    executable_candidates = [
        span.get("name", ""),
        attrs.get("process.executable", ""),
        attrs.get("executable", ""),
        attrs.get("exec.path", ""),
    ]
    if any(executable_basename(value) == "claude" for value in executable_candidates):
        return True
    return "\nclaude\n-p" in argv or "/claude\n-p" in argv


def is_claude_executable(span: dict, attrs: dict[str, str]) -> bool:
    executable_candidates = [
        span.get("name", ""),
        attrs.get("process.executable", ""),
        attrs.get("executable", ""),
        attrs.get("exec.path", ""),
    ]
    return any(executable_basename(value) == "claude" for value in executable_candidates)


def has_llm_request_for_pid(document: dict, pid: str) -> bool:
    if not pid:
        return False
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") == "llm.request" and attrs.get("process.pid") == pid:
            return True
    return False


def find_claude_llm_request_span(document: dict) -> dict | None:
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        return None
    exec_pid = span_attrs(exec_span).get("process.pid", "")
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get("process.pid") == exec_pid:
            return span
    return None


def find_claude_llm_tool_response_span(document: dict) -> dict | None:
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        return None
    exec_pid = span_attrs(exec_span).get("process.pid", "")
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.response":
            continue
        if attrs.get("process.pid") != exec_pid:
            continue
        if tool_calls_include_bash_marker(attrs.get("llm.response.tool_calls_json", "")):
            return span
    return None


def tool_calls_include_bash_marker(raw: str) -> bool:
    if not raw:
        return False
    try:
        tool_calls = json.loads(raw)
    except json.JSONDecodeError:
        return False
    if not isinstance(tool_calls, list):
        return False
    for call in tool_calls:
        function = call.get("function") if isinstance(call, dict) else None
        if not isinstance(function, dict):
            continue
        if function.get("name") == "Bash" and "ACTRAIL_AGENT_TREE_OK" in str(function.get("arguments", "")):
            return True
    return False


def find_claude_agent_command_span(document: dict) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get("invocation.kind") != "agent":
            continue
        child_executable = attrs.get("agent.child.executable", "")
        child_command = attrs.get("agent.child.command_line", "") or attrs.get("command.line", "")
        if executable_basename(child_executable) != "claude":
            continue
        if "-p" not in child_command:
            continue
        return span
    return None


def executable_basename(value: str) -> str:
    return Path(value).name if value else ""


def spans(document: dict) -> list[dict]:
    result: list[dict] = []
    for resource in document.get("resourceSpans", []):
        for scope in resource.get("scopeSpans", []):
            result.extend(scope.get("spans", []))
    return result


def span_attrs(span: dict) -> dict[str, str]:
    attrs: dict[str, str] = {}
    for attr in span.get("attributes", []):
        value = attr.get("value", {})
        if "stringValue" in value:
            attrs[attr.get("key", "")] = value["stringValue"]
        elif "intValue" in value:
            attrs[attr.get("key", "")] = str(value["intValue"])
    return attrs
