"""OTEL evidence checks for hidden agent invocation E2E."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class HiddenAgentProof:
    agent_a_pid: str
    xiaoo_pid: str
    script_b_parent_pid: str


def validate_hidden_agent_actions(
    document: dict,
    script_b_path: str,
    agent_a_path: str,
) -> HiddenAgentProof:
    agent_a = find_agent_a_exec(document, agent_a_path)
    if agent_a is None:
        raise RuntimeError("missing agent A process.exec")
    agent_a_attrs = span_attrs(agent_a)
    agent_a_pid = required_attr(agent_a_attrs, "process.pid")
    require_agent_identity(agent_a_attrs, "agent A")
    if find_llm_request_for_pid(document, agent_a_pid) is None:
        raise RuntimeError("missing agent A llm.request")

    xiaoo = find_xiaoo_exec_with_identity(document)
    if xiaoo is None:
        raise RuntimeError("missing xiaoo process.exec with agent identity")
    xiaoo_attrs = span_attrs(xiaoo)
    xiaoo_pid = required_attr(xiaoo_attrs, "process.pid")
    if find_llm_request_for_pid(document, xiaoo_pid) is None:
        raise RuntimeError("missing xiaoo llm.request")

    invocation = find_invocation_for_child_pid(document, xiaoo_pid)
    if invocation is None:
        raise RuntimeError("missing direct script B -> xiaoo agent.invocation")
    invocation_attrs = span_attrs(invocation)
    parent_pid = required_attr(invocation_attrs, "agent.parent.pid")
    if parent_pid == agent_a_pid:
        raise RuntimeError("agent.invocation incorrectly used ancestor agent A as parent")
    parent_command = invocation_attrs.get("agent.parent.command_line", "")
    if Path(script_b_path).name not in parent_command:
        raise RuntimeError(f"agent.invocation parent is not script B: {parent_command}")
    if find_invocation_with_parent_child(document, agent_a_pid, xiaoo_pid) is not None:
        raise RuntimeError("unexpected ancestor agent A -> xiaoo agent.invocation")
    return HiddenAgentProof(
        agent_a_pid=agent_a_pid,
        xiaoo_pid=xiaoo_pid,
        script_b_parent_pid=parent_pid,
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
    agent_a_pid = agent_a_attrs.get("process.pid", "")
    xiaoo_pid = span_attrs(xiaoo).get("process.pid", "")
    invocation = find_invocation_for_child_pid(document, xiaoo_pid)
    if invocation is None:
        return False
    invocation_attrs = span_attrs(invocation)
    return (
        bool(agent_a_pid)
        and bool(xiaoo_pid)
        and find_llm_request_for_pid(document, agent_a_pid) is not None
        and find_llm_request_for_pid(document, xiaoo_pid) is not None
        and invocation_attrs.get("agent.parent.pid") != agent_a_pid
        and Path(script_b_path).name in invocation_attrs.get("agent.parent.command_line", "")
    )


def describe_hidden_agent_evidence(document: dict, script_b_path: str, agent_a_path: str) -> str:
    agent_a = find_agent_a_exec(document, agent_a_path)
    xiaoo = find_xiaoo_exec_with_identity(document)
    agent_a_pid = span_attrs(agent_a).get("process.pid", "") if agent_a else ""
    xiaoo_pid = span_attrs(xiaoo).get("process.pid", "") if xiaoo else ""
    lines = [
        f"agent_a_exec={bool(agent_a)} pid={agent_a_pid} identity={identity_status(agent_a)}",
        f"xiaoo_exec={bool(xiaoo)} pid={xiaoo_pid} identity={identity_status(xiaoo)}",
        f"agent_a_llm={find_llm_request_for_pid(document, agent_a_pid) is not None}",
        f"xiaoo_llm={find_llm_request_for_pid(document, xiaoo_pid) is not None}",
    ]
    invocation = find_invocation_for_child_pid(document, xiaoo_pid)
    if invocation is None:
        lines.append("xiaoo_invocation=false")
    else:
        attrs = span_attrs(invocation)
        lines.append(
            "xiaoo_invocation=true "
            f"parent_pid={attrs.get('agent.parent.pid', '')} "
            f"parent_command_has_script_b={Path(script_b_path).name in attrs.get('agent.parent.command_line', '')}"
        )
    lines.append(f"llm_pids={','.join(llm_request_pids(document))}")
    lines.append(f"invocation_children={','.join(invocation_child_pids(document))}")
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


def find_llm_request_for_pid(document: dict, pid: str) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get("process.pid") == pid:
            return span
    return None


def find_invocation_for_child_pid(document: dict, child_pid: str) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "agent.invocation":
            continue
        if attrs.get("agent.child.pid") == child_pid:
            return span
    return None


def find_invocation_with_parent_child(
    document: dict,
    parent_pid: str,
    child_pid: str,
) -> dict | None:
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") != "agent.invocation":
            continue
        if attrs.get("agent.parent.pid") == parent_pid and attrs.get("agent.child.pid") == child_pid:
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


def llm_request_pids(document: dict) -> list[str]:
    pids = []
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") == "llm.request":
            pids.append(attrs.get("process.pid", ""))
    return pids


def invocation_child_pids(document: dict) -> list[str]:
    pids = []
    for span in spans(document):
        attrs = span_attrs(span)
        if attrs.get("actrail.action.kind") == "agent.invocation":
            pids.append(attrs.get("agent.child.pid", ""))
    return pids


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
