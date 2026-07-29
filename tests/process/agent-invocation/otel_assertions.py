from __future__ import annotations

from pathlib import Path

PROCESS_ID_ATTR = "actrail.process.id"
PARENT_PROCESS_ID_ATTR = "process.parent.id"


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
    identity_span = find_claude_identity_span(document)
    if identity_span is None:
        raise RuntimeError("missing claude agent.identity span")
    attrs = span_attrs(span)
    exec_attrs = span_attrs(exec_span)
    llm_attrs = span_attrs(llm_span)
    identity_attrs = span_attrs(identity_span)
    exec_process_id = exec_attrs.get(PROCESS_ID_ATTR, "")
    invocation_process_id = attrs.get(PROCESS_ID_ATTR, "")
    identity_process_id = identity_attrs.get(PROCESS_ID_ATTR, "")
    parent_process_id = attrs.get(PARENT_PROCESS_ID_ATTR, "")
    executable = attrs.get("process.executable", "") or attrs.get("executable", "")
    evidence_id = identity_attrs.get("agent.identity.evidence_action_id", "")
    llm_action_id = llm_attrs.get("actrail.action.id", "")
    if invocation_process_id != exec_process_id:
        raise RuntimeError(
            "agent command process ID "
            f"{invocation_process_id} does not match Claude process ID {exec_process_id}"
        )
    if identity_process_id != exec_process_id:
        raise RuntimeError("agent identity is not attached to the Claude process")
    if llm_attrs.get(PROCESS_ID_ATTR, "") != exec_process_id:
        raise RuntimeError("agent command evidence is not from the Claude child process")
    require_observed_identity(identity_attrs, "Claude")
    if not evidence_id or evidence_id != llm_action_id:
        raise RuntimeError(
            "agent identity evidence_action_id does not point to Claude llm.request: "
            f"identity={evidence_id}; llm={llm_action_id}"
        )
    if not parent_process_id or parent_process_id == exec_process_id:
        raise RuntimeError(
            "agent command parent process is not a direct external launcher: "
            f"{parent_process_id}"
        )
    if executable_basename(executable) != "claude":
        raise RuntimeError(f"agent command executable is not claude: {executable}")


def evidence_is_complete(document: dict) -> bool:
    return (
        find_claude_exec_span(document) is not None
        and find_claude_llm_request_span(document) is not None
        and find_claude_bash_command_span(document) is not None
        and find_claude_agent_command_span(document) is not None
        and find_claude_identity_span(document) is not None
    )


def find_claude_exec_span(document: dict) -> dict | None:
    identity_process_ids = {
        attrs.get(PROCESS_ID_ATTR, "")
        for span in spans(document)
        if (attrs := span_attrs(span)).get("actrail.action.kind") == "agent.identity"
        and attrs.get("agent.identity.status") == "observed"
    }
    candidates = [
        span
        for span in spans(document)
        if (attrs := span_attrs(span)).get("actrail.action.kind") == "process.exec"
        and is_claude_executable(span, attrs)
    ]
    return next(
        (
            span
            for span in candidates
            if span_attrs(span).get(PROCESS_ID_ATTR, "") in identity_process_ids
        ),
        candidates[0] if candidates else None,
    )


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


def find_claude_identity_span(document: dict) -> dict | None:
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        return None
    exec_process_id = span_attrs(exec_span).get(PROCESS_ID_ATTR, "")
    for span in spans(document):
        attrs = span_attrs(span)
        if (
            attrs.get("actrail.action.kind") == "agent.identity"
            and attrs.get(PROCESS_ID_ATTR) == exec_process_id
        ):
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
    exec_span = find_claude_exec_span(document)
    if exec_span is None:
        return None
    exec_process_id = span_attrs(exec_span).get(PROCESS_ID_ATTR, "")
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get(PROCESS_ID_ATTR) != exec_process_id:
            continue
        executable = attrs.get("process.executable", "") or attrs.get("executable", "")
        if executable_basename(executable) != "claude":
            continue
        return span
    return None


def require_observed_identity(attrs: dict[str, str], label: str) -> None:
    if attrs.get("agent.identity.status") != "observed":
        raise RuntimeError(f"{label} identity status is not observed")
    if attrs.get("agent.identity.source") != "llm.request":
        raise RuntimeError(f"{label} identity source is not llm.request")


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
