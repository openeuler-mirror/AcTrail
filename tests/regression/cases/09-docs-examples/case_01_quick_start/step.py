"""Docs example 01 quick-start regression step."""

from __future__ import annotations

import time
from pathlib import Path

from model import PASS, CaseResult
from workload_config import required

from helpers import (
    add_expected_found_check,
    communicate,
    event_rows,
    fail_step,
    line_evidence,
    network_rows,
    record_process_artifacts,
    run_clean,
    start_daemon,
    start_process,
    stop_process,
    track_add,
    viewer,
    write_stdin,
)


def run_quick_start(env, result: CaseResult, workload: dict[str, str]) -> str:
    name = "docs 01 quick-start"
    config = env.repo_root / "docs/examples/01.quick-start/operator.conf"
    script = env.repo_root / "docs/examples/01.quick-start/lifecycle_network_target.py"
    result.begin_check(name, "running attach workflow")
    daemon = None
    target = None
    try:
        run_clean(env, "quick-start", workload)
        daemon = start_daemon(env, config, workload)
        record_process_artifacts(result, daemon)
        process_env = {
            "ACTRAIL_TARGET_SOCKET_TIMEOUT_SECONDS": required(
                workload, "quick_start_socket_timeout_seconds"
            ),
            "ACTRAIL_TARGET_CHILD_HOLD_SECONDS": required(workload, "quick_start_child_hold_seconds"),
            "ACTRAIL_TARGET_POST_WORKLOAD_HOLD_SECONDS": required(
                workload, "quick_start_post_workload_hold_seconds"
            ),
        }
        target = start_process(
            env,
            [env.python, str(script), "--config", str(config)],
            extra_env=process_env,
        )
        trace_id = track_add(env, config, target.pid, "docs-quick-start", workload)
        write_stdin(target, "\n")
        stdout, stderr = communicate(target, float(required(workload, "quick_start_workload_timeout_seconds")))
        if "workload complete" not in stdout:
            raise RuntimeError(f"quick-start workload did not complete\nstdout={stdout}\nstderr={stderr}")
        summary, events, network = wait_quick_start_views(env, config, trace_id, workload)
        result.stdout_tail = env.output_tail("\n".join((summary, events, network)))
        add_expected_found_check(
            result,
            f"{name} trace completion",
            "trace state Completed",
            f"trace-{trace_id}; {line_evidence(summary, 'state=Completed')}",
            "summary/process/network viewer output contains the documented lifecycle and loopback TCP evidence",
        )
        add_expected_found_check(
            result,
            f"{name} process lifecycle",
            "Process fork, exec, and exit rows",
            event_rows(
                events,
                [
                    ("fork row", "Process", "fork", ()),
                    ("exec row", "Process", "exec", ()),
                    ("exit row", "Process", "exit", ()),
                ],
            ),
            "quick-start documents child process lifecycle observation",
        )
        add_expected_found_check(
            result,
            f"{name} loopback network",
            "connect, accept, send, and recv network rows",
            network_rows(
                network,
                [
                    ("connect row", "connect", ()),
                    ("accept row", "accept", ()),
                    ("send row", "send", ()),
                    ("recv row", "recv", ()),
                ],
            ),
            "quick-start documents local TCP roundtrip observation",
        )
        return PASS
    except Exception as error:
        return fail_step(env, result, name, error)
    finally:
        stop_process(target, workload)
        stop_process(daemon, workload)


def wait_quick_start_views(env, config: Path, trace_id: int, workload: dict[str, str]) -> tuple[str, str, str]:
    for _ in range(int(required(workload, "drain_attempts"))):
        summary = viewer(env, config, "summary", trace_id)
        events = viewer(env, config, "events", trace_id)
        network = viewer(env, config, "network", trace_id)
        if (
            "Completed" in summary
            and all(fragment in events for fragment in ("Process", "fork", "exec", "exit"))
            and all(fragment in network for fragment in ("connect", "accept", "send", "recv"))
        ):
            return summary, events, network
        time.sleep(float(required(workload, "drain_sleep_seconds")))
    raise RuntimeError("quick-start viewer output missed process lifecycle or loopback network rows")
