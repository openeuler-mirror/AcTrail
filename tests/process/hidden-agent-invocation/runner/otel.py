"""OTEL evidence checks for hidden agent invocation E2E."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


PROCESS_ID_ATTR = "actrail.process.id"
PARENT_PROCESS_ID_ATTR = "process.parent.id"
CHILD_PROCESS_ID_ATTR = "agent.child.process_id"


@dataclass(frozen=True)
class HiddenAgentProof:
    agent_a_process_id: str
    xiaoo_process_id: str
    script_b_parent_process_id: str


def validate_hidden_agent_actions(
    document: dict,
    script_b_path: str,
    agent_a_path: str,
) -> HiddenAgentProof:
    agent_a = find_agent_a_exec(document, agent_a_path)
    if agent_a is None:
        raise RuntimeError("missing agent A process.exec")
    agent_a_attrs = span_attrs(agent_a)
    agent_a_process_id = required_attr(agent_a_attrs, PROCESS_ID_ATTR)
    require_agent_identity(agent_a_attrs, "agent A")
    if find_llm_request_for_process(document, agent_a_process_id) is None:
        raise RuntimeError("missing agent A llm.request")

    xiaoo = find_xiaoo_exec_with_identity(document)
    if xiaoo is None:
        raise RuntimeError("missing xiaoo process.exec with agent identity")
    xiaoo_attrs = span_attrs(xiaoo)
    xiaoo_process_id = required_attr(xiaoo_attrs, PROCESS_ID_ATTR)
    if find_llm_request_for_process(document, xiaoo_process_id) is None:
        raise RuntimeError("missing xiaoo llm.request")

    invocation = find_agent_command_for_child_process(document, xiaoo_process_id)
    if invocation is None:
        raise RuntimeError("missing direct script B -> xiaoo agent command")
    invocation_attrs = span_attrs(invocation)
    parent_process_id = required_attr(invocation_attrs, PARENT_PROCESS_ID_ATTR)
    if parent_process_id == agent_a_process_id:
        raise RuntimeError("agent command incorrectly used ancestor agent A as parent")
    parent_command = process_command_line(document, parent_process_id)
    if Path(script_b_path).name not in parent_command:
        raise RuntimeError(f"agent command parent is not script B: {parent_command}")
    if find_agent_command_with_parent_child(
        document, agent_a_process_id, xiaoo_process_id
    ) is not None:
        raise RuntimeError("unexpected ancestor agent A -> xiaoo agent command")
    return HiddenAgentProof(
        agent_a_process_id=agent_a_process_id,
        xiaoo_process_id=xiaoo_process_id,
        script_b_parent_process_id=parent_process_id,
    )


def hidden_agent_evidence_is_complete(
    document: dict,
    script_b_path: str,
    agent_a_path: str,
) -> bool:
    agent_a = find_agent_a_exec(document, agent_a_path)
    xiaoo = find_xiaoo_exec_with_identity(document)
    if agent_a is None or xiaoo is None:
        return False
    agent_a_attrs = span_attrs(agent_a)
    if agent_a_attrs.get("agent.identity.status") != "observed":
        return False
    if agent_a_attrs.get("agent.identity.source") != "llm.request":
        return False
    agent_a_process_id = agent_a_attrs.get(PROCESS_ID_ATTR, "")
    xiaoo_process_id = span_attrs(xiaoo).get(PROCESS_ID_ATTR, "")
    invocation = find_agent_command_for_child_process(document, xiaoo_process_id)
    if invocation is None:
        return False
    invocation_attrs = span_attrs(invocation)
    parent_process_id = invocation_attrs.get(PARENT_PROCESS_ID_ATTR, "")
    return (
        bool(agent_a_process_id)
        and bool(xiaoo_process_id)
        and find_llm_request_for_process(document, agent_a_process_id) is not None
        and find_llm_request_for_process(document, xiaoo_process_id) is not None
        and parent_process_id != agent_a_process_id
        and Path(script_b_path).name in process_command_line(document, parent_process_id)
    )


def describe_hidden_agent_evidence(document: dict, script_b_path: str, agent_a_path: str) -> str:
    agent_a = find_agent_a_exec(document, agent_a_path)
    xiaoo = find_xiaoo_exec_with_identity(document)
    agent_a_process_id = span_attrs(agent_a).get(PROCESS_ID_ATTR, "") if agent_a else ""
    xiaoo_process_id = span_attrs(xiaoo).get(PROCESS_ID_ATTR, "") if xiaoo else ""
    lines = [
        f"agent_a_exec={bool(agent_a)} process_id={agent_a_process_id} identity={identity_status(agent_a)}",
        f"xiaoo_exec={bool(xiaoo)} process_id={xiaoo_process_id} identity={identity_status(xiaoo)}",
        f"agent_a_llm={find_llm_request_for_process(document, agent_a_process_id) is not None}",
        f"xiaoo_llm={find_llm_request_for_process(document, xiaoo_process_id) is not None}",
    ]
    invocation = find_agent_command_for_child_process(document, xiaoo_process_id)
    if invocation is None:
        lines.append("xiaoo_invocation=false")
    else:
        attrs = span_attrs(invocation)
        parent_process_id = attrs.get(PARENT_PROCESS_ID_ATTR, "")
        lines.append(
            "xiaoo_invocation=true "
            f"parent_process_id={parent_process_id} "
            f"parent_command_has_script_b={Path(script_b_path).name in process_command_line(document, parent_process_id)}"
        )
    lines.append(f"llm_process_ids={','.join(llm_request_process_ids(document))}")
    lines.append(
        f"invocation_child_process_ids={','.join(agent_command_child_process_ids(document))}"
    )
    return "\n".join(lines)


def find_agent_a_exec(document: dict, agent_a_path: str) -> dict | None:
    expected_basename = Path(agent_a_path).name
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "process.exec":
            continue
        executable = attrs.get("process.executable", attrs.get("executable", ""))
        if executable_basename(executable) == expected_basename:
            return span
        command_line = attrs.get("command_line", "")
        argv = attrs.get("argv", "")
        if expected_basename in command_line or expected_basename in argv:
            return span
    return None


def find_xiaoo_exec_with_identity(document: dict) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "process.exec":
            continue
        executable = attrs.get("process.executable", attrs.get("executable", ""))
        if executable_basename(executable) != "xiaoo":
            continue
        if attrs.get("agent.identity.status") == "observed":
            return span
    return None


def find_llm_request_for_process(document: dict, process_id: str) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get(PROCESS_ID_ATTR) == process_id:
            return span
    return None


def find_agent_command_for_child_process(document: dict, child_process_id: str) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get("invocation.kind") != "agent":
            continue
        if attrs.get(CHILD_PROCESS_ID_ATTR) == child_process_id:
            return span
    return None


def find_agent_command_with_parent_child(
    document: dict,
    parent_process_id: str,
    child_process_id: str,
) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get("invocation.kind") != "agent":
            continue
        if (
            attrs.get(PARENT_PROCESS_ID_ATTR) == parent_process_id
            and attrs.get(CHILD_PROCESS_ID_ATTR) == child_process_id
        ):
            return span
    return None


def require_agent_identity(attrs: dict[str, str], label: str) -> None:
    if attrs.get("agent.identity.status") != "observed":
        raise RuntimeError(f"{label} was not marked as an observed agent")
    if attrs.get("agent.identity.source") != "llm.request":
        raise RuntimeError(f"{label} agent identity was not sourced from llm.request")


def identity_status(span: dict | None) -> str:
    if span is None:
        return "missing"
    attrs = span_attrs(span)
    return attrs.get("agent.identity.status", "none")


def llm_request_process_ids(document: dict) -> list[str]:
    process_ids = []
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") == "llm.request":
            process_ids.append(attrs.get(PROCESS_ID_ATTR, ""))
    return process_ids


def agent_command_child_process_ids(document: dict) -> list[str]:
    process_ids = []
    for span in spans(document):
        attrs = span_attrs(span)
        if (
            attrs.get("actrail.action.kind") == "command.invocation"
            and attrs.get("invocation.kind") == "agent"
        ):
            process_ids.append(attrs.get(CHILD_PROCESS_ID_ATTR, ""))
    return process_ids


def process_command_line(document: dict, process_id: str) -> str:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "process.exec":
            continue
        if attrs.get(PROCESS_ID_ATTR) == process_id:
            return attrs.get("command_line", "")
    return ""


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


def required_attr(attrs: dict[str, str], key: str) -> str:
    value = attrs.get(key)
    if not value:
        raise RuntimeError(f"missing span attribute {key}")
    return value


def executable_basename(value: str) -> str:
    return Path(value).name if value else ""
