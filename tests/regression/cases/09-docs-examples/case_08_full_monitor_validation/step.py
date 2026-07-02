"""Docs example 08 full-monitor launch regression step."""

from __future__ import annotations

from model import PASS, CaseResult
from workload_config import required

from helpers import (
    add_expected_found_check,
    actrail_command,
    fail_step,
    line_evidence,
    parse_trace_id,
    run_clean,
    run_command,
    start_daemon,
    stop_process,
    viewer,
)


def run_full_monitor_python_launch(env, result: CaseResult, workload: dict[str, str]) -> str:
    name = "docs 08 full-monitor Python launch"
    config = env.repo_root / "docs/examples/08.full-monitor-validation/operator.conf"
    result.begin_check(name, "running launch workflow with sync TLS audit enabled")
    daemon = None
    try:
        run_clean(env, "full-monitor", workload)
        daemon = start_daemon(env, config, workload)
        completed = run_command(
            env,
            actrail_command(
                env,
                "actrailctl",
                config,
                "launch",
                "--seccomp-notify",
                "disabled",
                "--name",
                "docs-full-monitor-python",
                "--",
                env.python,
                "-c",
                "print('ACTRAIL_FULL_MONITOR_PYTHON_OK')",
            ),
            float(required(workload, "quick_start_workload_timeout_seconds")),
        )
        output = completed.output
        trace_id = parse_trace_id(output)
        summary = viewer(env, config, "summary", trace_id)
        result.stdout_tail = env.output_tail("\n".join((output, summary)))
        add_expected_found_check(
            result,
            f"{name} process completed",
            "Python launch prints sentinel and trace reaches Exited",
            "\n".join(
                (
                    line_evidence(output, "ACTRAIL_FULL_MONITOR_PYTHON_OK"),
                    line_evidence(summary, "state=Exited"),
                )
            ),
            "full-monitor config exercises launch-mode sync TLS audit injection on a simple Python process",
        )
        return PASS
    except Exception as error:
        return fail_step(env, result, name, error)
    finally:
        stop_process(daemon, workload)
