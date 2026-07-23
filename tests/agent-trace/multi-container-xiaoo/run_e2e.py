#!/usr/bin/env python3
"""Validate concurrent eBPF and LLM-call capture in two Docker containers."""

from __future__ import annotations

import argparse
import json
import os
import select
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


RUNTIME_TOKEN = "@RUNTIME_DIR@"
LOCAL_API_KEY_ENV = "ACTRAIL_MULTI_CONTAINER_XIAOO_API_KEY"
LOCAL_API_KEY = "actrail-multi-container-local-key"
PROFILE_NAME = "multi-container-xiaoo"


@dataclass(frozen=True)
class Workload:
    suffix: str
    trace_name: str
    request_marker: str
    response_marker: str
    task_prompt: str
    write_marker: str
    container_name: str
    config_path: Path
    input_path: Path
    output_path: Path
    input_text: str
    hold_seconds: float


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument(
        "--image",
        default=os.environ.get(
            "ACTRAIL_MULTI_CONTAINER_IMAGE",
            "openeuler/openeuler:24.03-lts-sp3",
        ),
    )
    parser.add_argument(
        "--xiaoo-bin",
        default=os.environ.get("XIAOO_BINARY", "/root/.cargo/bin/xiaoo"),
    )
    parser.add_argument("--operator-template", default=str(case_dir / "operator.conf"))
    parser.add_argument(
        "--seccomp-profile",
        default=str(
            case_dir.parents[2]
            / "deploy/container-auto/seccomp/actrail-notify.json"
        ),
    )
    parser.add_argument("--container-start-stagger-seconds", type=float, default=10.0)
    parser.add_argument("--active-overlap-seconds", type=float, default=3.0)
    parser.add_argument("--ready-timeout-seconds", type=float, default=30.0)
    parser.add_argument("--launch-timeout-seconds", type=float, default=180.0)
    parser.add_argument("--drain-timeout-seconds", type=float, default=30.0)
    parser.add_argument("--keep-runtime", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    require_root()
    case_dir = Path(__file__).resolve().parent
    repo = case_dir.parents[2]
    bin_dir = resolve_path(args.bin_dir, repo)
    actraild = require_executable(bin_dir / "actraild")
    actrailctl = require_executable(bin_dir / "actrailctl")
    actrailviewer = require_executable(bin_dir / "actrailviewer")
    tls_runtime = require_file(bin_dir / "libactrail_tls_payload_probe_sync.so")
    xiaoo = require_executable(resolve_path(args.xiaoo_bin, repo))
    operator_template = require_file(resolve_path(args.operator_template, repo))
    workload_script = require_file(case_dir / "workload.sh")
    docker_seccomp = resolve_docker_seccomp(args.seccomp_profile, repo)
    provider_script = require_file(repo / "tests/support/llm-http-proxy/provider_proxy.py")
    require_command("docker")

    runtime = Path(tempfile.mkdtemp(prefix="actrail-multi-container-xiaoo.", dir="/tmp"))
    if args.container_start_stagger_seconds < 0 or args.active_overlap_seconds < 0:
        raise RuntimeError("container stagger and active overlap must be non-negative")
    run_id = f"{int(time.time())}-{os.getpid()}"
    label = f"io.actrail.multi-container-e2e={run_id}"
    workloads = [
        Workload(
            suffix="a",
            trace_name="container-a-release-summary",
            request_marker="ACTRAIL_TASK_A_SUMMARIZE_RELEASE_NOTES",
            response_marker="ACTRAIL_TASK_A_RELEASE_SUMMARY_COMPLETE",
            task_prompt=(
                "Task A: summarize release-note input prepared by the file workload. "
                "Return ACTRAIL_TASK_A_SUMMARIZE_RELEASE_NOTES."
            ),
            write_marker="ACTRAIL_TASK_A_FILE_WRITE_COMPLETE",
            container_name=f"actrail-multi-xiaoo-a-{run_id}",
            config_path=runtime / "xiaoo-a.toml",
            input_path=runtime / "tasks/a-release-notes-input.txt",
            output_path=runtime / "tasks/a-release-summary-output.txt",
            input_text=(
                "Release task A\n"
                "- add multi-container PID namespace attribution\n"
                "- retain independent eBPF and LLM traces\n"
            ),
            hold_seconds=(
                args.container_start_stagger_seconds + args.active_overlap_seconds
            ),
        ),
        Workload(
            suffix="b",
            trace_name="container-b-security-review",
            request_marker="ACTRAIL_TASK_B_REVIEW_SECURITY_POLICY",
            response_marker="ACTRAIL_TASK_B_SECURITY_REVIEW_COMPLETE",
            task_prompt=(
                "Task B: review the security-policy input prepared by the file workload. "
                "Return ACTRAIL_TASK_B_REVIEW_SECURITY_POLICY."
            ),
            write_marker="ACTRAIL_TASK_B_FILE_WRITE_COMPLETE",
            container_name=f"actrail-multi-xiaoo-b-{run_id}",
            config_path=runtime / "xiaoo-b.toml",
            input_path=runtime / "tasks/b-security-policy-input.txt",
            output_path=runtime / "tasks/b-security-review-output.txt",
            input_text=(
                "Security task B\n"
                "- authenticate shared Unix-socket peers with SO_PEERCRED\n"
                "- reject cross-container trace control and payload injection\n"
            ),
            hold_seconds=0.0,
        ),
    ]
    config = runtime / "operator.conf"
    database = runtime / "data/actrail.sqlite"
    daemon_log = runtime / "log/daemon.stderr"
    provider_processes: list[subprocess.Popen[str]] = []
    launches: list[subprocess.Popen[str]] = []
    daemon: subprocess.Popen[str] | None = None
    succeeded = False

    try:
        prepare_runtime(runtime, operator_template, config)
        prepare_workload_files(workloads)
        provider_urls = []
        for workload in workloads:
            provider, listen_url = start_provider(
                provider_script,
                workload.response_marker,
                args.ready_timeout_seconds,
                repo,
            )
            provider_processes.append(provider)
            provider_urls.append(rewrite_provider_host(listen_url, "127.0.0.1"))
        for workload, provider_url in zip(workloads, provider_urls):
            write_xiaoo_config(workload.config_path, provider_url)

        daemon = start_daemon(actraild, config, daemon_log)
        wait_for_daemon(actrailctl, config, daemon, args.ready_timeout_seconds)

        launches.append(
            start_container(
                args,
                workloads[0],
                label,
                runtime,
                config,
                actrailctl,
                tls_runtime,
                xiaoo,
                workload_script,
                docker_seccomp,
            )
        )
        wait_for_active_traces(
            database,
            1,
            args.ready_timeout_seconds,
            launches,
            workloads[:1],
        )
        if args.container_start_stagger_seconds > 0:
            time.sleep(args.container_start_stagger_seconds)
        launches.append(
            start_container(
                args,
                workloads[1],
                label,
                runtime,
                config,
                actrailctl,
                tls_runtime,
                xiaoo,
                workload_script,
                docker_seccomp,
            )
        )

        trace_rows = wait_for_active_traces(
            database,
            len(workloads),
            args.ready_timeout_seconds,
            launches,
            workloads,
        )
        container_ids = {
            workload.container_name: inspect_container_id(workload.container_name)
            for workload in workloads
        }
        require_trace_container_isolation(trace_rows, set(container_ids.values()))
        require_trace_display_names(database, workloads)
        require_trace_start_stagger(
            database,
            args.container_start_stagger_seconds,
        )

        outputs = wait_for_launches(launches, workloads, args.launch_timeout_seconds)
        for workload in workloads:
            (runtime / f"container-{workload.suffix}.stdout").write_text(
                outputs[workload.suffix],
                encoding="utf-8",
            )
        wait_for_completed_traces(database, len(workloads), args.drain_timeout_seconds)
        trace_by_workload = wait_for_trace_evidence(
            database,
            actrailviewer,
            config,
            workloads,
            args.drain_timeout_seconds,
        )
        verify_outputs(outputs, workloads)
        verify_file_outputs(workloads)
        verify_no_cross_trace_llm_actions(
            actrailviewer,
            config,
            trace_by_workload,
            workloads,
        )

        for workload in workloads:
            trace_id = trace_by_workload[workload.suffix]
            print(
                "multi_container_trace "
                f"container={workload.container_name} "
                f"container_id={container_ids[workload.container_name]} "
                f"trace=trace-{trace_id} "
                f"trace_name={workload.trace_name} "
                f"input={workload.input_path} "
                f"output={workload.output_path} "
                f"request_marker={workload.request_marker} "
                f"response_marker={workload.response_marker}"
            )
        print("multi-container xiaoO eBPF and llm.call E2E complete")
        succeeded = True
        return 0
    finally:
        for launch in launches:
            terminate_process(launch)
        remove_owned_containers(label)
        if daemon is not None:
            terminate_process(daemon)
            print_process_stderr("daemon", daemon)
        for provider in provider_processes:
            terminate_process(provider)
            print_process_stderr("provider", provider)
        if args.keep_runtime:
            print(
                f"multi_container_runtime_preserved={runtime} succeeded={succeeded}",
                file=sys.stderr,
            )
        else:
            shutil.rmtree(runtime, ignore_errors=True)


def prepare_runtime(runtime: Path, template: Path, config: Path) -> None:
    for child in ("run", "data", "data/export", "log"):
        (runtime / child).mkdir(parents=True, exist_ok=True)
    rendered = template.read_text(encoding="utf-8").replace(RUNTIME_TOKEN, str(runtime))
    if RUNTIME_TOKEN in rendered:
        raise RuntimeError("operator template still contains an unresolved runtime token")
    config.write_text(rendered, encoding="utf-8")


def prepare_workload_files(workloads: list[Workload]) -> None:
    for workload in workloads:
        workload.input_path.parent.mkdir(parents=True, exist_ok=True)
        workload.input_path.write_text(workload.input_text, encoding="utf-8")


def start_provider(
    script: Path,
    response_marker: str,
    timeout: float,
    cwd: Path,
) -> tuple[subprocess.Popen[str], str]:
    process = subprocess.Popen(
        [
            sys.executable,
            str(script),
            "--mode",
            "local-stream",
            "--bind-host",
            "0.0.0.0",
            "--bind-port",
            "0",
            "--local-stream-response-text",
            response_marker,
            "--local-stream-reasoning-tokens",
            "3",
            "--response-chunk-delay-seconds",
            "0.05",
        ],
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    deadline = time.monotonic() + timeout
    if process.stdout is None:
        raise RuntimeError("provider stdout is unavailable")
    while time.monotonic() < deadline:
        remaining = max(0.0, deadline - time.monotonic())
        readable, _, _ = select.select([process.stdout], [], [], remaining)
        if not readable:
            break
        line = process.stdout.readline()
        if line.startswith("proxy_base_url="):
            return process, line.split("=", 1)[1].strip()
        if process.poll() is not None:
            raise RuntimeError(f"provider exited early: {read_process_stderr(process)}")
    raise RuntimeError("provider did not report its listen URL")


def rewrite_provider_host(url: str, gateway: str) -> str:
    parsed = urlsplit(url)
    port = parsed.port
    if port is None:
        raise RuntimeError(f"provider URL has no port: {url}")
    return urlunsplit((parsed.scheme, f"{gateway}:{port}", parsed.path, "", ""))


def write_xiaoo_config(path: Path, provider_url: str) -> None:
    path.write_text(
        "\n".join(
            [
                "[llm]",
                'provider = "deepseek"',
                'model = "deepseek-chat"',
                f'api_key_env = "{LOCAL_API_KEY_ENV}"',
                f'api_base = "{provider_url}"',
                "max_tokens = 128",
                "context_window = 32768",
                'reasoning_effort = "off"',
                "",
            ]
        ),
        encoding="utf-8",
    )


def start_daemon(
    actraild: Path,
    config: Path,
    daemon_log: Path,
) -> subprocess.Popen[str]:
    log = daemon_log.open("w", encoding="utf-8")
    return subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=log,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )


def wait_for_daemon(
    actrailctl: Path,
    config: Path,
    daemon: subprocess.Popen[str],
    timeout: float,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if daemon.poll() is not None:
            raise RuntimeError(f"actraild exited early: {read_process_stderr(daemon)}")
        result = subprocess.run(
            [str(actrailctl), "--config", str(config), "doctor"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode == 0 and "ebpf" in result.stdout:
            return
        time.sleep(0.1)
    raise RuntimeError("actraild did not become ready with eBPF")


def start_container(
    args: argparse.Namespace,
    workload: Workload,
    label: str,
    runtime: Path,
    config: Path,
    actrailctl: Path,
    tls_runtime: Path,
    xiaoo: Path,
    workload_script: Path,
    docker_seccomp: str,
) -> subprocess.Popen[str]:
    command = [
        "docker",
        "run",
        "--name",
        workload.container_name,
        "--label",
        label,
        "--user",
        "0:0",
        "--network",
        "host",
        "--security-opt",
        docker_seccomp,
        "-e",
        f"{LOCAL_API_KEY_ENV}={LOCAL_API_KEY}",
        "-e",
        f"ACTRAIL_XIAOO_CONFIG={workload.config_path}",
        "-e",
        f"ACTRAIL_XIAOO_PROMPT={workload.task_prompt}",
        "-e",
        f"ACTRAIL_TASK_INPUT={workload.input_path}",
        "-e",
        f"ACTRAIL_TASK_OUTPUT={workload.output_path}",
        "-e",
        f"ACTRAIL_TASK_WRITE_MARKER={workload.write_marker}",
        "-e",
        f"ACTRAIL_TASK_HOLD_SECONDS={workload.hold_seconds}",
        "-v",
        f"{runtime}:{runtime}",
        "-v",
        f"{actrailctl}:/usr/local/bin/actrailctl:ro",
        "-v",
        f"{tls_runtime}:/usr/local/bin/libactrail_tls_payload_probe_sync.so:ro",
        "-v",
        f"{xiaoo}:/root/.cargo/bin/xiaoo:ro",
        "-v",
        f"{workload_script}:/usr/local/bin/actrail-multi-workload:ro",
        "--entrypoint",
        "/usr/local/bin/actrailctl",
        args.image,
        "--config",
        str(config),
        "launch",
        "--name",
        workload.trace_name,
        "--host-ebpf",
        "required",
        "--seccomp-notify",
        "required",
        "--",
        "/bin/sh",
        "/usr/local/bin/actrail-multi-workload",
    ]
    return subprocess.Popen(
        command,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def wait_for_active_traces(
    database: Path,
    expected: int,
    timeout: float,
    launches: list[subprocess.Popen[str]],
    workloads: list[Workload],
) -> list[tuple[int, str, str | None]]:
    deadline = time.monotonic() + timeout
    last_rows: list[tuple[int, str, str | None]] = []
    while time.monotonic() < deadline:
        for launch, workload in zip(launches, workloads):
            if launch.poll() is not None:
                stdout, stderr = launch.communicate()
                raise RuntimeError(
                    f"{workload.container_name} exited before traces became active "
                    f"exit={launch.returncode} stdout={stdout} stderr={stderr}"
                )
        if database.exists():
            with sqlite3.connect(database) as connection:
                last_rows = connection.execute(
                    """
                    SELECT trace_id, lifecycle_state, root_container_id
                    FROM traces
                    WHERE profile_name LIKE ?
                    ORDER BY trace_id
                    """,
                    (f"{PROFILE_NAME}%",),
                ).fetchall()
            if len(last_rows) == expected and all(row[1] == "active" for row in last_rows):
                return last_rows
        time.sleep(0.05)
    raise RuntimeError(f"expected traces were not simultaneously active: {last_rows}")


def resolve_docker_seccomp(value: str, repo: Path) -> str:
    if value == "unconfined":
        return "seccomp=unconfined"
    return f"seccomp={require_file(resolve_path(value, repo))}"


def require_trace_container_isolation(
    trace_rows: list[tuple[int, str, str | None]],
    expected_container_ids: set[str],
) -> None:
    trace_container_ids = {row[2] for row in trace_rows}
    if None in trace_container_ids:
        raise RuntimeError(f"a trace has no Docker container identity: {trace_rows}")
    if trace_container_ids != expected_container_ids:
        raise RuntimeError(
            "trace/container identity mismatch: "
            f"traces={trace_container_ids} containers={expected_container_ids}"
        )


def require_trace_display_names(database: Path, workloads: list[Workload]) -> None:
    with sqlite3.connect(database) as connection:
        rows = connection.execute(
            """
            SELECT display_name
            FROM traces
            WHERE profile_name LIKE ?
            ORDER BY trace_id
            """,
            (f"{PROFILE_NAME}%",),
        ).fetchall()
    actual = {str(row[0]) for row in rows}
    expected = {workload.trace_name for workload in workloads}
    if actual != expected:
        raise RuntimeError(f"trace display names mismatch: actual={actual} expected={expected}")


def require_trace_start_stagger(database: Path, expected_seconds: float) -> None:
    if expected_seconds <= 0:
        return
    with sqlite3.connect(database) as connection:
        rows = connection.execute(
            """
            SELECT trace_id, created_at
            FROM traces
            WHERE profile_name LIKE ?
            ORDER BY created_at
            """,
            (f"{PROFILE_NAME}%",),
        ).fetchall()
    if len(rows) != 2:
        raise RuntimeError(f"cannot verify trace start stagger: {rows}")
    actual_seconds = (int(rows[1][1]) - int(rows[0][1])) / 1_000_000_000
    minimum_seconds = max(0.0, expected_seconds - 0.5)
    if actual_seconds < minimum_seconds:
        raise RuntimeError(
            "trace start stagger is too short: "
            f"actual={actual_seconds:.3f}s expected={expected_seconds:.3f}s"
        )


def wait_for_launches(
    launches: list[subprocess.Popen[str]],
    workloads: list[Workload],
    timeout: float,
) -> dict[str, str]:
    deadline = time.monotonic() + timeout
    outputs: dict[str, str] = {}
    for launch, workload in zip(launches, workloads):
        remaining = max(0.1, deadline - time.monotonic())
        try:
            stdout, stderr = launch.communicate(timeout=remaining)
        except subprocess.TimeoutExpired as error:
            raise RuntimeError(f"{workload.container_name} did not finish") from error
        if launch.returncode != 0:
            raise RuntimeError(
                f"{workload.container_name} failed exit={launch.returncode} "
                f"stdout={stdout} stderr={stderr}"
            )
        outputs[workload.suffix] = stdout
    return outputs


def wait_for_completed_traces(database: Path, expected: int, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    last_rows: list[tuple[int, str]] = []
    while time.monotonic() < deadline:
        with sqlite3.connect(database) as connection:
            last_rows = connection.execute(
                """
                SELECT trace_id, lifecycle_state
                FROM traces
                WHERE profile_name LIKE ?
                ORDER BY trace_id
                """,
                (f"{PROFILE_NAME}%",),
            ).fetchall()
        if len(last_rows) == expected and all(
            state in ("completed", "exited") for _, state in last_rows
        ):
            return
        time.sleep(0.1)
    raise RuntimeError(f"traces did not complete: {last_rows}")


def wait_for_trace_evidence(
    database: Path,
    actrailviewer: Path,
    config: Path,
    workloads: list[Workload],
    timeout: float,
) -> dict[str, int]:
    deadline = time.monotonic() + timeout
    last_error = "no evidence query was run"
    while time.monotonic() < deadline:
        try:
            trace_by_workload = identify_traces_by_request_marker(
                database,
                actrailviewer,
                config,
                workloads,
            )
            for workload in workloads:
                trace_id = trace_by_workload[workload.suffix]
                require_ebpf_evidence(database, trace_id)
                require_llm_actions(actrailviewer, config, trace_id)
                require_file_actions(actrailviewer, config, trace_id, workload)
            return trace_by_workload
        except RuntimeError as error:
            last_error = str(error)
            time.sleep(0.2)
    raise RuntimeError(f"trace evidence did not become complete: {last_error}")


def identify_traces_by_request_marker(
    database: Path,
    actrailviewer: Path,
    config: Path,
    workloads: list[Workload],
) -> dict[str, int]:
    result: dict[str, int] = {}
    with sqlite3.connect(database) as connection:
        trace_ids = [
            int(row[0])
            for row in connection.execute(
                """
                SELECT trace_id
                FROM traces
                WHERE profile_name LIKE ?
                ORDER BY trace_id
                """,
                (f"{PROFILE_NAME}%",),
            ).fetchall()
        ]
    actions_by_trace = {
        trace_id: load_trace_actions(actrailviewer, config, trace_id)
        for trace_id in trace_ids
    }
    for workload in workloads:
        matches = [
            trace_id
            for trace_id, actions in actions_by_trace.items()
            if any(
                action.get("kind") == "llm.request"
                and workload.request_marker
                in action.get("attributes", {}).get("llm.request.message_preview", "")
                for action in actions
            )
        ]
        if len(matches) != 1:
            raise RuntimeError(
                f"request marker {workload.request_marker} matched traces {matches}"
            )
        result[workload.suffix] = matches[0]
    if len(set(result.values())) != len(workloads):
        raise RuntimeError(f"workload markers collapsed into one trace: {result}")
    return result


def require_ebpf_evidence(database: Path, trace_id: int) -> None:
    with sqlite3.connect(database) as connection:
        process_count = connection.execute(
            "SELECT COUNT(*) FROM events WHERE trace_id = ? AND collector = 'ebpf' AND kind = 'process'",
            (trace_id,),
        ).fetchone()[0]
        network_count = connection.execute(
            """
            SELECT COUNT(*)
            FROM events
            WHERE trace_id = ?
              AND collector = 'ebpf'
              AND kind IN ('net', 'network')
            """,
            (trace_id,),
        ).fetchone()[0]
    if process_count < 1 or network_count < 1:
        raise RuntimeError(
            f"trace-{trace_id} missing eBPF evidence "
            f"process={process_count} network={network_count}"
        )


def require_llm_actions(actrailviewer: Path, config: Path, trace_id: int) -> None:
    actions = load_trace_actions(actrailviewer, config, trace_id)
    kinds = {action.get("kind") for action in actions}
    required = {"llm.call", "llm.request", "llm.response"}
    missing = required - kinds
    if missing:
        raise RuntimeError(f"trace-{trace_id} missing semantic actions {sorted(missing)}")
    failed_responses = [
        action
        for action in actions
        if action.get("kind") == "llm.response" and action.get("status") != "success"
    ]
    if failed_responses:
        raise RuntimeError(f"trace-{trace_id} contains failed llm.response actions")


def require_file_actions(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
    workload: Workload,
) -> None:
    actions = load_trace_actions(actrailviewer, config, trace_id)
    expectations = [
        ("file.read", str(workload.input_path), "file.bytes_read"),
        ("file.write", str(workload.output_path), "file.bytes_written"),
    ]
    for kind, path, bytes_attribute in expectations:
        matches = [
            action
            for action in actions
            if action.get("kind") == kind
            and action.get("attributes", {}).get("file.path") == path
            and positive_integer(
                action.get("attributes", {}).get(bytes_attribute)
            )
        ]
        if not matches:
            raise RuntimeError(
                f"trace-{trace_id} missing {kind} for {path} with positive "
                f"{bytes_attribute}"
            )


def positive_integer(value: object) -> bool:
    try:
        return int(str(value)) > 0
    except (TypeError, ValueError):
        return False


def load_trace_actions(
    actrailviewer: Path,
    config: Path,
    trace_id: int,
) -> list[dict[str, object]]:
    output = run_checked(
        [
            str(actrailviewer),
            "--config",
            str(config),
            "--output-format",
            "json",
            "actions",
            "--trace-id",
            f"trace-{trace_id}",
        ]
    )
    document = json.loads(output)
    actions = document.get("actions")
    if not isinstance(actions, list):
        raise RuntimeError(f"trace-{trace_id} viewer output has no actions array")
    return actions


def verify_outputs(outputs: dict[str, str], workloads: list[Workload]) -> None:
    for workload in workloads:
        output = outputs[workload.suffix]
        if workload.response_marker not in output:
            raise RuntimeError(
                f"{workload.container_name} output lacks {workload.response_marker}: {output}"
            )


def verify_file_outputs(workloads: list[Workload]) -> None:
    for workload in workloads:
        if not workload.output_path.is_file():
            raise RuntimeError(f"missing workload output: {workload.output_path}")
        output = workload.output_path.read_text(encoding="utf-8")
        if workload.input_text not in output or workload.write_marker not in output:
            raise RuntimeError(
                f"workload output does not contain its input and write marker: "
                f"{workload.output_path}"
            )


def verify_no_cross_trace_llm_actions(
    actrailviewer: Path,
    config: Path,
    trace_by_workload: dict[str, int],
    workloads: list[Workload],
) -> None:
    for workload in workloads:
        trace_id = trace_by_workload[workload.suffix]
        actions = load_trace_actions(actrailviewer, config, trace_id)
        own_responses = [
            action
            for action in actions
            if action.get("kind") == "llm.response"
            and workload.response_marker
            in action.get("attributes", {}).get("llm.response.content_text", "")
        ]
        if not own_responses:
            raise RuntimeError(
                f"trace-{trace_id} lacks response marker {workload.response_marker}"
            )
        for other in workloads:
            if other.suffix == workload.suffix:
                continue
            for action in actions:
                attributes = action.get("attributes", {})
                request_preview = attributes.get("llm.request.message_preview", "")
                response_text = attributes.get("llm.response.content_text", "")
                if (
                    other.request_marker in request_preview
                    or other.response_marker in response_text
                ):
                    raise RuntimeError(
                        f"trace-{trace_id} contains LLM markers owned by container "
                        f"{other.suffix}"
                    )


def inspect_container_id(name: str) -> str:
    return run_checked(["docker", "inspect", "--format", "{{.Id}}", name]).strip()


def remove_owned_containers(label: str) -> None:
    result = subprocess.run(
        ["docker", "ps", "-aq", "--filter", f"label={label}"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    ids = result.stdout.split()
    if ids:
        subprocess.run(
            ["docker", "rm", "-f", *ids],
            text=True,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            check=False,
        )


def terminate_process(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    try:
        process.terminate()
    except ProcessLookupError:
        return
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        process.kill()
        process.wait(timeout=5)


def print_process_stderr(label: str, process: subprocess.Popen[str]) -> None:
    stderr = read_process_stderr(process)
    if stderr:
        print(f"{label}_stderr:\n{stderr}", file=sys.stderr)


def read_process_stderr(process: subprocess.Popen[str]) -> str:
    if process.stderr is None:
        return ""
    return process.stderr.read()


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("run as root")


def require_command(name: str) -> None:
    if shutil.which(name) is None:
        raise RuntimeError(f"missing command: {name}")


def resolve_path(value: str, repo: Path) -> Path:
    path = Path(value).expanduser()
    return path.resolve() if path.is_absolute() else (repo / path).resolve()


def require_executable(path: Path) -> Path:
    if not path.is_file() or not os.access(path, os.X_OK):
        raise RuntimeError(f"missing executable: {path}")
    return path


def require_file(path: Path) -> Path:
    if not path.is_file():
        raise RuntimeError(f"missing file: {path}")
    return path


def run_checked(command: list[str]) -> str:
    result = subprocess.run(
        command,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed exit={result.returncode}: {' '.join(command)}\n{result.stderr}"
        )
    return result.stdout


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"multi-container xiaoO E2E failed: {error}", file=sys.stderr)
        raise SystemExit(1)
