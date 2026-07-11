from __future__ import annotations

from pathlib import Path

PROCESS_ID_ATTR = "actrail.process.id"
PARENT_PROCESS_ID_ATTR = "process.parent.id"
CHILD_PROCESS_ID_ATTR = "agent.child.process_id"


def require_claude_exec_span(document: dict) -> None:
    span = find_claude_exec_span(document)
    if span is None:
        raise RuntimeError("missing claude process.exec span")
    attrs = span_attrs(span)
    if not attrs.get(PROCESS_ID_ATTR) or not is_claude_executable(span, attrs):
        raise RuntimeError("claude process.exec span has no logical process identity")


def require_claude_llm_request_span(document: dict) -> None:
    if find_claude_llm_request_span(document) is None:
        raise RuntimeError("missing claude llm.request span")


def require_claude_bash_command_span(document: dict) -> None:
    if find_claude_bash_command_span(document) is None:
        raise RuntimeError("missing successful Claude child Bash command span")


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
    exec_process_id = exec_attrs.get(PROCESS_ID_ATTR, "")
    parent_process_id = attrs.get(PARENT_PROCESS_ID_ATTR, "")
    child_process_id = attrs.get(CHILD_PROCESS_ID_ATTR, "")
    child_executable = attrs.get("agent.child.executable", "")
    trigger = attrs.get("agent.invocation.trigger", "")
    evidence_id = attrs.get("agent.invocation.evidence_action_id", "")
    llm_action_id = llm_attrs.get("actrail.action.id", "")
    if child_process_id != exec_process_id:
        raise RuntimeError(
            "agent command child process ID "
            f"{child_process_id} does not match Claude process ID {exec_process_id}"
        )
    if llm_attrs.get(PROCESS_ID_ATTR, "") != child_process_id:
        raise RuntimeError("agent command evidence is not from the Claude child process")
    if trigger != "child_llm_request":
        raise RuntimeError(f"agent command trigger is not child_llm_request: {trigger}")
    if not evidence_id or evidence_id != llm_action_id:
        raise RuntimeError(
            "agent command evidence_action_id does not point to Claude llm.request: "
            f"edge={evidence_id}; llm={llm_action_id}"
        )
    if not parent_process_id or parent_process_id == child_process_id:
        raise RuntimeError(
            "agent command parent process is not a direct external launcher: "
            f"{parent_process_id}"
        )
    if executable_basename(child_executable) != "claude":
        raise RuntimeError(f"agent command child is not claude: {child_executable}")


def evidence_is_complete(document: dict) -> bool:
    return (
        find_claude_exec_span(document) is not None
        and find_claude_llm_request_span(document) is not None
        and find_claude_bash_command_span(document) is not None
        and find_claude_agent_command_span(document) is not None
    )


def find_claude_exec_span(document: dict) -> dict | None:
    agent_command = find_claude_agent_command_span(document)
    child_process_id = (
        span_attrs(agent_command).get(CHILD_PROCESS_ID_ATTR, "") if agent_command else ""
    )
    for span in spans(document):
        attrs = span_attrs(span)
        if (
            attrs.get("actrail.action.kind") == "process.exec"
            and attrs.get(PROCESS_ID_ATTR) == child_process_id
            and is_claude_executable(span, attrs)
        ):
            return span
    for span in spans(document):
        attrs = span_attrs(span)
        if (
            attrs.get("actrail.action.kind") == "process.exec"
            and attrs.get("agent.identity.status") == "observed"
            and is_claude_executable(span, attrs)
        ):
            return span
    return None


def is_claude_executable(span: dict, attrs: dict[str, str]) -> bool:
    executable_candidates = [
        span.get("name", ""),
        attrs.get("process.executable", ""),
        attrs.get("executable", ""),
        attrs.get("exec.path", ""),
    ]
    return any(executable_basename(value) == "claude" for value in executable_candidates)


def find_claude_llm_request_span(document: dict) -> dict | None:
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        return None
    exec_process_id = span_attrs(exec_span).get(PROCESS_ID_ATTR, "")
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get(PROCESS_ID_ATTR) == exec_process_id:
            return span
    return None


def find_claude_bash_command_span(document: dict) -> dict | None:
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        return None
    exec_process_id = span_attrs(exec_span).get(PROCESS_ID_ATTR, "")
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get("actrail.action.status") != "success":
            continue
        if attrs.get("actrail.action.completeness") != "complete":
            continue
        if attrs.get(PARENT_PROCESS_ID_ATTR) != exec_process_id:
            continue
        executable = executable_basename(
            attrs.get("process.executable", "") or attrs.get("executable", "")
        )
        if executable in {"bash", "sh"}:
            return span
    return None


def find_claude_agent_command_span(document: dict) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get("invocation.kind") != "agent":
            continue
        child_executable = attrs.get("agent.child.executable", "")
        if executable_basename(child_executable) != "claude":
            continue
        if not attrs.get(CHILD_PROCESS_ID_ATTR):
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
