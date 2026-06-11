"""Docs example 03 extended observation regression step."""

from __future__ import annotations

import time
from pathlib import Path

from model import PASS, CaseResult
from workload_config import read_config, required

from helpers import (
    add_expected_found_check,
    actrail_command,
    clean_default_operator_state,
    communicate,
    evidence_rows,
    event_rows,
    fail_step,
    line_evidence,
    network_rows,
    parse_trace_id,
    prefixed_line_evidence,
    record_process_artifacts,
    run_clean,
    run_command,
    start_daemon,
    start_process,
    stop_process,
    viewer,
    wait_for_output,
    write_stdin,
)


def run_extended_observation(env, result: CaseResult, workload: dict[str, str]) -> str:
    name = "docs 03 extended observation"
    config = None
    workload_config = env.repo_root / "docs/examples/03.extended-observation-e2e/workload.conf"
    target_values = read_config(workload_config)
    result.begin_check(name, "running launch workflow")
    daemon = None
    target = None
    try:
        run_clean(env, "extended-observation", workload)
        clean_default_operator_state(env, workload)
        daemon = start_daemon(env, config, workload)
        record_process_artifacts(result, daemon)
        target = start_process(
            env,
            actrail_command(
                env,
                "actrailctl",
                config,
                "launch",
                "--name",
                "docs-extended-observation",
                "--",
                str(env.release_binary("ebpf_probe")),
                "workload",
                "--config",
                str(workload_config),
            ),
        )
        output = wait_for_output(
            target,
            "waiting_for=" + required(target_values, "stdio_stdin_message"),
            float(required(workload, "extended_workload_ready_timeout_seconds")),
        )
        trace_id = parse_trace_id(output)
        time.sleep(float(required(workload, "extended_resource_sample_sleep_seconds")))
        write_stdin(target, required(target_values, "stdio_stdin_message") + "\n")
        wait_for_output(
            target,
            "waiting_for=" + required(target_values, "stdio_continue_message"),
            float(required(workload, "extended_workload_timeout_seconds")),
        )
        write_stdin(target, required(target_values, "stdio_continue_message") + "\n")
        stdout, stderr = communicate(target, float(required(workload, "extended_workload_timeout_seconds")))
        if target.returncode != 0:
            raise RuntimeError(f"extended workload failed\nstdout={stdout}\nstderr={stderr}")
        summary, events, network, payloads = wait_extended_views(env, config, trace_id, workload, target_values)
        result.stdout_tail = env.output_tail("\n".join((summary, events, network, payloads)))
        add_expected_found_check(
            result,
            f"{name} manual trace completion",
            "trace state Completed",
            f"trace-{trace_id}; {line_evidence(summary, 'state=Completed')}",
            "viewer output contains documented process/file/network/resource/stdio evidence",
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
            "extended docs require process lifecycle observation",
        )
        add_expected_found_check(
            result,
            f"{name} file and mmap",
            "fifo/file path, mmap_shared, mkdir, rename, unlink, truncate",
            event_rows(
                events,
                [
                    ("fifo open row", "File", "open", (required(target_values, "fifo_path"),)),
                    ("file write row", "File", "write", (required(target_values, "file_path"),)),
                    ("mmap row", "File", "mmap_shared", ()),
                    ("mkdir row", "File", "mkdir", ()),
                    ("rename row", "File", "rename", ()),
                    ("unlink row", "File", "unlink", ()),
                    ("truncate row", "File", "truncate", ()),
                ],
            ),
            "extended docs require file, path mutation, and mmap evidence",
        )
        add_expected_found_check(
            result,
            f"{name} network",
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
            "extended docs require local TCP network evidence",
        )
        add_expected_found_check(
            result,
            f"{name} resource and stdio",
            "Resource process_tree event and Stdio payload rows",
            event_rows(
                events,
                [
                    ("resource process_tree row", "Resource", "process_tree", ()),
                ],
            )
            + evidence_rows(
                payloads,
                [
                    ("stdio outbound payload row", ("Stdio", "outbound", "write")),
                    ("stdio inbound payload row", ("Stdio", "inbound", "read")),
                ],
            ),
            "extended docs require resource metrics and stdio payload evidence",
        )
        verify_live_output = run_extended_verify_live(env, result, workload)
        result.stdout_tail = env.output_tail(result.stdout_tail + "\n" + verify_live_output)
        return PASS
    except Exception as error:
        return fail_step(env, result, name, error)
    finally:
        stop_process(target, workload)
        stop_process(daemon, workload)


def wait_extended_views(
    env,
    config: Path | None,
    trace_id: int,
    workload: dict[str, str],
    target_values: dict[str, str],
) -> tuple[str, str, str, str]:
    for _ in range(int(required(workload, "drain_attempts"))):
        summary = viewer(env, config, "summary", trace_id)
        events = viewer(env, config, "events", trace_id, "--head", required(workload, "extended_events_head"))
        network = viewer(env, config, "network", trace_id, "--head", required(workload, "extended_network_head"))
        payloads = viewer(env, config, "payloads", trace_id, "--head", required(workload, "extended_payload_head"))
        event_fragments = (
            "Process",
            "File",
            required(target_values, "fifo_path"),
            required(target_values, "file_path"),
            "mmap_shared",
            "mkdir",
            "rename",
            "unlink",
            "truncate",
            "Net",
            "Resource",
            "process_tree",
        )
        if (
            "Completed" in summary
            and all(fragment in events for fragment in event_fragments)
            and all(fragment in network for fragment in ("connect", "accept", "send", "recv"))
            and "Stdio" in payloads
        ):
            return summary, events, network, payloads
        time.sleep(float(required(workload, "drain_sleep_seconds")))
    raise RuntimeError("extended observation viewer output missed documented evidence rows")


def run_extended_verify_live(env, result: CaseResult, workload: dict[str, str]) -> str:
    name = "docs 03 extended observation verify-live"
    result.begin_check(name, "running maintainer assertion pass")
    run_clean(env, "extended-observation", workload)
    completed = run_command(
        env,
        [
            str(env.release_binary("ebpf_probe")),
            "verify-live",
            "--config",
            str(env.repo_root / "docs/examples/03.extended-observation-e2e/observation.conf"),
        ],
        float(required(workload, "extended_verify_live_timeout_seconds")),
    )
    output = completed.output
    add_expected_found_check(
        result,
        f"{name} process/file/net",
        "live verification passed with process/file/net events",
        prefixed_line_evidence(
            output,
            (
                "live verification passed",
                "process_events=",
                "file_events=",
                "net_events=",
            ),
        ),
        "maintainer assertion pass covers the extended observation event families",
    )
    add_expected_found_check(
        result,
        f"{name} ipc/provider/stdio",
        "IPC fifo/pipe/unix_socket, provider actrail-local-tcp, stdio payloads",
        prefixed_line_evidence(
            output,
            (
                "ipc_events=",
                "provider_events=",
                "stdio_payloads=",
            ),
        ),
        "verify-live must include the docs IPC/provider/stdio checks that are part of extended observation",
    )
    return output
