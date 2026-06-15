"""Docs example 07 xiaoO-to-Claude regression step."""

from __future__ import annotations

import shlex
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

from .edge_validation import validate_agent_invocation_edge


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
        [
            claude,
            *shlex.split(docs_workload.get("claude_extra_args", "")),
            "-p",
            required(docs_workload, "direct_claude_prompt"),
        ],
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
            required(docs_workload, "xiaoo_availability_prompt"),
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
        docs_workload.get("claude_extra_args", ""),
    )
    otel_status = PASS if not edge.missing else FAIL
    add_expected_found_check(
        result,
        f"{name} OTEL invocation edge",
        "one complete agent-labeled command.invocation plus Claude tool-use llm.response",
        bullet_evidence(edge.facts + [f"missing: {item}" for item in edge.missing]),
        "docs example 07 must prove the direct launcher -> Claude semantic edge plus the xiaoO trace root",
        status=otel_status,
    )
    if otel_status == FAIL:
        return FAIL
    return status


def script_completion_evidence(output: str, fragments: tuple[str, ...]) -> str:
    return bullet_evidence([line_evidence(output, fragment) for fragment in fragments])
