#!/usr/bin/env python3
"""Run the docs xiaoO -> Java LangChain4j agent invocation E2E."""

from __future__ import annotations

import argparse
import os
import shlex
import shutil
import sys
import time
from dataclasses import dataclass
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "_common"))
import java_langchain4j as java_workload  # noqa: E402

sys.path.insert(0, str(Path(__file__).resolve().parents[3] / "tests/agent-trace"))
from common import (  # noqa: E402
    clean_configured_paths,
    export_otel,
    launch_and_parse_trace_with_daemon,
    otel_attrs,
    otel_spans,
    read_config,
    repo_root,
    require_binary,
    require_complete_llm_action,
    require_complete_payload_rows_any,
    require_root,
    required,
    run_checked,
    start_daemon,
    stop_process,
    wait_for_actions,
    wait_for_payloads_any,
)


JAVA_COMPLETION_MARKER = "ACTRAIL_LANGCHAIN4J_AGENT_COMPLETE"


@dataclass(frozen=True)
class InvocationEdge:
    root_pid: str
    parent_pid: str
    child_pid: str
    evidence_action_id: str
    llm_action_id: str


def main() -> int:
    args = parse_args()
    require_root()
    repo = repo_root()
    java_project_dir = repo / "docs/examples/_workloads/java-langchain4j-agent"
    workload_path = resolve_path(args.workload_config, repo)
    workload = read_config(workload_path)

    xiaoo = resolve_agent_command(required(workload, "agent_command"))
    java_workload.require_tool("timeout")
    java = java_workload.require_tool("java")
    javac = java_workload.require_tool("javac")
    mvn = java_workload.require_tool("mvn")
    java_workload.require_java_major([java, "-version"], "java")
    java_workload.require_java_major([javac, "-version"], "javac")
    java_workload.require_java_major([mvn, "--version"], "Maven Java runtime")

    llm = java_workload.resolve_llm_settings(workload, required)
    if not os.environ.get(llm.api_key_env):
        raise RuntimeError(f"missing environment variable {llm.api_key_env}")
    java_workload.require_https_provider(llm.base_url)

    java_workload.prepare_maven_project(
        mvn,
        java_project_dir,
        float(required(workload, "maven_build_timeout_seconds")),
        run_checked,
    )
    fat_jar = java_workload.require_fat_jar(java_project_dir)
    java_child_argv = java_workload.java_argv(java, fat_jar, llm)
    prompt = render_workload_prompt(workload, workload_path, java_child_argv)

    bin_dir = resolve_path(args.bin_dir, repo)
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    actrailviewer = require_binary(bin_dir, "actrailviewer")
    config = resolve_path(args.config, repo)

    clean_configured_paths(actrailctl, config)
    daemon = start_daemon(
        actraild,
        config,
        float(required(workload, "daemon_ready_timeout_seconds")),
    )
    try:
        trace_id, output = launch_and_parse_trace_with_daemon(
            daemon,
            actrailctl,
            config,
            "xiaoo-java-langchain4j-agent",
            [xiaoo, "run", "-p", prompt],
            float(required(workload, "launch_timeout_seconds")),
            float(required(workload, "launch_poll_interval_seconds")),
            float(required(workload, "process_stop_timeout_seconds")),
        )
        java_workload.require_workload_answer(output, llm, JAVA_COMPLETION_MARKER)
        payloads = wait_for_payloads_any(
            actrailctl,
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            required(workload, "payload_head"),
            accepted_payload_fragments(),
        )
        payload_count = require_complete_payload_rows_any(
            payloads,
            accepted_payload_sources(),
            direction="outbound",
        )
        actions = wait_for_actions(
            actrailviewer,
            config,
            trace_id,
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
        )
        require_complete_llm_action(actions)
        otel = wait_for_java_agent_invocation_otel(
            actrailviewer,
            config,
            trace_id,
            Path(required(workload, "otel_output_path")),
            int(required(workload, "drain_attempts")),
            float(required(workload, "drain_sleep_seconds")),
            xiaoo,
            java,
            fat_jar,
            llm.model,
            llm.prompt,
        )
        edge = require_java_agent_invocation_edge(
            otel,
            xiaoo,
            java,
            fat_jar,
            llm.model,
            llm.prompt,
        )
        emit_java_llm_otel_evidence(
            otel,
            edge.child_pid,
            llm.model,
            llm.prompt,
            int(required(workload, "evidence_text_max_chars")),
        )
        print(f"xiaoo_java_langchain4j_trace_id={trace_id}")
        print(f"xiaoo_java_langchain4j_root_pid={edge.root_pid}")
        print(f"xiaoo_java_langchain4j_parent_pid={edge.parent_pid}")
        print(f"xiaoo_java_langchain4j_child_pid={edge.child_pid}")
        print(f"xiaoo_java_langchain4j_payload_segments={payload_count}")
        print(f"xiaoo_java_langchain4j_evidence_action_id={edge.evidence_action_id}")
        print(f"xiaoo_java_langchain4j_otel={required(workload, 'otel_output_path')}")
        print("xiaoO Java LangChain4j agent invocation docs e2e complete")
    finally:
        stop_process(daemon, float(required(workload, "daemon_stop_timeout_seconds")))
    return 0


def parse_args() -> argparse.Namespace:
    example_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(example_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(example_dir / "workload.conf"))
    return parser.parse_args()


def resolve_agent_command(raw: str) -> str:
    path = Path(raw)
    if path.parent != Path("."):
        if not path.exists():
            raise RuntimeError(f"configured agent_command does not exist: {raw}")
        if not os.access(path, os.X_OK):
            raise RuntimeError(f"configured agent_command is not executable: {raw}")
        return str(path)
    resolved = shutil.which(raw)
    if resolved is None:
        raise RuntimeError(f"configured agent_command is not on PATH: {raw}")
    return resolved


def render_workload_prompt(values: dict[str, str], workload_path: Path, java_argv: list[str]) -> str:
    template_path = resolve_config_path(required(values, "agent_prompt_template"), workload_path)
    template_values = dict(values)
    template_values["java_command_line"] = shlex.join(java_argv)
    template = template_path.read_text(encoding="utf-8").strip()
    try:
        return template.format_map(template_values)
    except KeyError as error:
        raise RuntimeError(f"missing prompt template value: {error}") from error


def resolve_config_path(raw: str, workload_path: Path) -> Path:
    path = Path(raw)
    if path.is_absolute():
        return path
    cwd_path = Path.cwd() / path
    if cwd_path.exists():
        return cwd_path
    return workload_path.resolve().parent / path


def accepted_payload_sources() -> list[tuple[str, str]]:
    return [
        ("TlsUserSpace", "jsse"),
    ]


def accepted_payload_fragments() -> list[list[str]]:
    return [
        [source, library, "outbound", "Complete", "success"]
        for source, library in accepted_payload_sources()
    ]


def wait_for_java_agent_invocation_otel(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    output_path: Path,
    attempts: int,
    sleep_sec: float,
    xiaoo: str,
    java: str,
    fat_jar: Path,
    model: str,
    prompt: str,
) -> dict:
    last_error: Exception | None = None
    for _ in range(attempts):
        document = export_otel(actrailviewer, config, trace_id, output_path)
        try:
            require_java_agent_invocation_edge(document, xiaoo, java, fat_jar, model, prompt)
            return document
        except RuntimeError as error:
            last_error = error
        time.sleep(sleep_sec)
    raise RuntimeError(f"OTEL export did not contain complete xiaoO -> Java agent evidence: {last_error}")


def require_java_agent_invocation_edge(
    document: dict,
    xiaoo: str,
    java: str,
    fat_jar: Path,
    model: str,
    prompt: str,
) -> InvocationEdge:
    root_exec = find_process_exec(document, xiaoo, None)
    if root_exec is None:
        raise RuntimeError("missing xiaoO process.exec span")
    child_exec = find_process_exec(document, java, fat_jar.name)
    if child_exec is None:
        raise RuntimeError("missing Java LangChain4j process.exec span")
    root_attrs = otel_attrs(root_exec)
    child_attrs = otel_attrs(child_exec)
    child_pid = required_attr(child_attrs, "process.pid", "Java process.exec")
    llm_span = find_llm_request_for_pid(document, child_pid, model, prompt)
    if llm_span is None:
        raise RuntimeError("missing Java child llm.request span with configured model and prompt")
    llm_attrs = otel_attrs(llm_span)
    command_span = find_agent_command_for_child(document, child_pid, fat_jar.name)
    if command_span is None:
        raise RuntimeError("missing agent-labeled command.invocation for Java child")
    command_attrs = otel_attrs(command_span)
    parent_pid = required_attr(command_attrs, "process.parent.pid", "Java agent command.invocation")
    command_child_pid = required_attr(command_attrs, "agent.child.pid", "Java agent command.invocation")
    evidence_action_id = required_attr(
        command_attrs,
        "agent.invocation.evidence_action_id",
        "Java agent command.invocation",
    )
    llm_action_id = required_attr(llm_attrs, "actrail.action.id", "Java child llm.request")

    if command_child_pid != child_pid:
        raise RuntimeError(
            f"agent command child pid {command_child_pid} does not match Java exec pid {child_pid}"
        )
    if llm_attrs.get("process.pid", "") != child_pid:
        raise RuntimeError("Java child llm.request uses a different pid")
    if command_attrs.get("agent.invocation.trigger") != "child_llm_request":
        raise RuntimeError(
            "agent command trigger is not child_llm_request: "
            f"{command_attrs.get('agent.invocation.trigger')}"
        )
    if evidence_action_id != llm_action_id:
        raise RuntimeError(
            "agent command evidence_action_id does not point to Java child llm.request: "
            f"edge={evidence_action_id}; llm={llm_action_id}"
        )
    if not parent_pid or parent_pid == child_pid:
        raise RuntimeError(f"agent command parent pid is not the direct external launcher: {parent_pid}")
    if span_trace_id(root_exec) != span_trace_id(child_exec) or span_trace_id(child_exec) != span_trace_id(command_span):
        raise RuntimeError("xiaoO exec, Java exec, and agent command spans are not in the same trace")

    return InvocationEdge(
        root_pid=required_attr(root_attrs, "process.pid", "xiaoO process.exec"),
        parent_pid=parent_pid,
        child_pid=child_pid,
        evidence_action_id=evidence_action_id,
        llm_action_id=llm_action_id,
    )


def find_process_exec(document: dict, expected_executable: str, command_fragment: str | None) -> dict | None:
    expected_name = Path(expected_executable).name
    fallback: list[dict] = []
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") != "process.exec":
            continue
        command_line = attrs.get("command_line", "") or attrs.get("command.line", "")
        argv = attrs.get("argv", "")
        if command_fragment is not None and command_fragment not in command_line and command_fragment not in argv:
            continue
        if executable_basename(span, attrs) == expected_name:
            return span
        if expected_name in command_line or f"\n{expected_name}\n" in argv:
            fallback.append(span)
    return fallback[0] if fallback else None


def find_llm_request_for_pid(document: dict, pid: str, model: str, prompt: str) -> dict | None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") != "llm.request":
            continue
        if attrs.get("process.pid") != pid:
            continue
        if attrs.get("actrail.action.completeness") != "complete":
            continue
        if attrs.get("actrail.action.status") != "success":
            continue
        body = attrs.get("http.request.body_text") or attrs.get("llm.request.payload_text", "")
        if attrs.get("llm.request.model") == model and prompt in body:
            return span
    return None


def find_agent_command_for_child(document: dict, child_pid: str, command_fragment: str) -> dict | None:
    for span in otel_spans(document):
        attrs = otel_attrs(span)
        if attrs.get("actrail.action.kind") != "command.invocation":
            continue
        if attrs.get("invocation.kind") != "agent":
            continue
        child_command = attrs.get("agent.child.command_line", "") or attrs.get("command.line", "")
        if attrs.get("agent.child.pid") == child_pid and command_fragment in child_command:
            return span
    return None


def emit_java_llm_otel_evidence(
    document: dict,
    child_pid: str,
    model: str,
    prompt: str,
    max_text_chars: int,
) -> None:
    span = find_llm_request_for_pid(document, child_pid, model, prompt)
    if span is None:
        print("evidence.java_llm_request=not exported")
        return
    attrs = otel_attrs(span)
    body = attrs.get("http.request.body_text") or attrs.get("llm.request.payload_text", "")
    print(f"evidence.java_llm_request.action_id={attrs.get('actrail.action.id', '')}")
    print(f"evidence.java_llm_request.model={attrs.get('llm.request.model', '')}")
    print(f"evidence.java_llm_request.source={attrs.get('payload.source_boundary', '')}")
    print(f"evidence.java_llm_request.payload_bytes={attrs.get('llm.request.payload_bytes', '')}")
    print(f"evidence.java_llm_request.body_has_prompt={str(prompt in body).lower()}")
    print(f"evidence.java_llm_request.body_preview={clip_text(body, max_text_chars)}")


def clip_text(text: str, max_chars: int) -> str:
    if max_chars < 0:
        raise RuntimeError("evidence text max chars must be non-negative")
    return text if len(text) <= max_chars else text[:max_chars] + "...[truncated]"


def executable_basename(span: dict, attrs: dict[str, str]) -> str:
    for value in (
        attrs.get("process.executable", ""),
        attrs.get("executable", ""),
        attrs.get("exec.path", ""),
        span.get("name", ""),
    ):
        if value:
            return Path(value).name
    return ""


def required_attr(attrs: dict[str, str], key: str, label: str) -> str:
    value = attrs.get(key, "")
    if not value:
        raise RuntimeError(f"{label} is missing {key}")
    return value


def span_trace_id(span: dict) -> str:
    return str(span.get("traceId", ""))


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"xiaoO Java LangChain4j agent invocation docs e2e failed: {error}", file=sys.stderr)
        raise SystemExit(1)
