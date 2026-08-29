#!/usr/bin/env python3
"""Validate concurrent eBPF and LLM-call capture in two Docker containers."""

from __future__ import annotations

import argparse
import json
import os
import re
import select
import signal
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
import zlib
from dataclasses import dataclass
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit


REPO = Path(__file__).resolve().parents[5]
CONTAINER_MANAGER_DIR = (
    REPO
    / "tests"
    / "v2"
    / "common"
    / "test_suites"
    / "container-manager"
)
sys.path.insert(0, str(CONTAINER_MANAGER_DIR))

from container import TestContainer  # noqa: E402
from image import ContainerImage  # noqa: E402
from request import ContainerRequest  # noqa: E402


RUNTIME_TOKEN = "@RUNTIME_DIR@"
LOCAL_API_KEY_ENV = "ACTRAIL_MULTI_CONTAINER_XIAOO_API_KEY"
LOCAL_API_KEY = "actrail-multi-container-local-key"
PROFILE_NAME = "multi-container-xiaoo"
IDENTITY_HANDSHAKE_TRACE_NAME = "existing-process-identity-handshake"
NONLEADER_EXEC_TRACE_NAME = "nonleader-thread-exec"
BUSINESS_TRACE_NAMES = (
    "container-a-release-summary",
    "container-b-security-review",
)


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
        default=str(case_dir / "seccomp/actrail-notify.json"),
    )
    parser.add_argument("--container-start-stagger-seconds", type=float, default=10.0)
    parser.add_argument("--active-overlap-seconds", type=float, default=3.0)
    parser.add_argument("--ready-timeout-seconds", type=float, default=30.0)
    parser.add_argument("--launch-timeout-seconds", type=float, default=180.0)
    parser.add_argument("--drain-timeout-seconds", type=float, default=30.0)
    parser.add_argument("--rebuild-image", action="store_true")
    parser.add_argument("--keep-runtime", action="store_true")
    parser.add_argument("--keep-runtime-on-failure", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    require_root()
    case_dir = Path(__file__).resolve().parent
    repo = REPO
    bin_dir = resolve_path(args.bin_dir, repo)
    actraild = require_executable(bin_dir / "actraild")
    actrailctl = require_executable(bin_dir / "actrailctl")
    actrailviewer = require_executable(bin_dir / "actrailviewer")
    tls_runtime = require_file(bin_dir / "libactrail_tls_payload_probe_sync.so")
    xiaoo = require_executable(resolve_path(args.xiaoo_bin, repo))
    operator_template = require_file(resolve_path(args.operator_template, repo))
    workload_script = require_file(case_dir / "workload.sh")
    image_dockerfile = require_file(case_dir / "Dockerfile")
    docker_seccomp = resolve_docker_seccomp(args.seccomp_profile, repo)
    provider_script = require_file(repo / "tests/support/llm-http-proxy/provider_proxy.py")
    nonleader_exec_script = require_file(case_dir / "nonleader_exec.py")
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
            hold_seconds=args.active_overlap_seconds,
        ),
    ]
    config = runtime / "operator.conf"
    database = runtime / "data/actrail.sqlite"
    daemon_log = runtime / "log/daemon.stderr"
    provider_processes: list[subprocess.Popen[str]] = []
    launches: list[subprocess.Popen[str]] = []
    containers: list[TestContainer] = []
    daemon: subprocess.Popen[str] | None = None
    test_image: ContainerImage | None = None
    succeeded = False
    failure_stage = "runtime_preparation"

    try:
        prepare_runtime(runtime, operator_template, config, actraild)
        prepare_workload_files(workloads)
        failure_stage = "container_image_preparation"
        test_image = prepare_agent_image(
            runtime,
            args.image,
            image_dockerfile,
            args.rebuild_image,
        )
        provider_urls = []
        failure_stage = "provider_startup"
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

        failure_stage = "daemon_startup"
        daemon = start_daemon(actraild, config, daemon_log)
        wait_for_daemon(
            actrailctl,
            config,
            daemon,
            daemon_log,
            args.ready_timeout_seconds,
        )
        failure_stage = "existing_process_identity_handshake"
        verify_existing_process_identity_handshake(
            actrailctl,
            config,
            args.ready_timeout_seconds,
        )
        failure_stage = "nonleader_exec_validation"
        verify_nonleader_exec_lifecycle(
            actrailctl,
            config,
            database,
            nonleader_exec_script,
            runtime / "nonleader-exec.marker",
            args.ready_timeout_seconds,
        )

        failure_stage = "container_startup"
        for workload in workloads:
            container = TestContainer(
                ContainerRequest(
                    image=test_image,
                    name=workload.container_name,
                    labels=(label,),
                    volumes=(
                        f"{runtime}:{runtime}",
                        f"{actrailctl}:/usr/local/bin/actrailctl:ro",
                        f"{tls_runtime}:"
                        "/usr/local/bin/libactrail_tls_payload_probe_sync.so:ro",
                        f"{xiaoo}:/root/.cargo/bin/xiaoo:ro",
                        f"{workload_script}:"
                        "/usr/local/bin/actrail-multi-workload:ro",
                    ),
                    security_options=(docker_seccomp,),
                    user="0:0",
                    network="host",
                    ready_timeout_seconds=args.ready_timeout_seconds,
                )
            ).start()
            containers.append(container)
        pid_namespaces = {
            workload.container_name: container.pid_namespace()
            for workload, container in zip(workloads, containers)
        }
        require_distinct_pid_namespaces(pid_namespaces)
        container_ids = {container.container_id for container in containers}

        failure_stage = "workload_launch"
        launches.append(
            start_workload(
                containers[0],
                workloads[0],
                config,
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
            start_workload(
                containers[1],
                workloads[1],
                config,
            )
        )

        failure_stage = "active_trace_validation"
        trace_rows = wait_for_active_traces(
            database,
            len(workloads),
            args.ready_timeout_seconds,
            launches,
            workloads,
        )
        require_trace_container_isolation(trace_rows, container_ids)
        require_trace_display_names(database, workloads)
        require_trace_start_stagger(
            database,
            args.container_start_stagger_seconds,
        )

        failure_stage = "workload_completion"
        outputs = wait_for_launches(launches, workloads, args.launch_timeout_seconds)
        for workload in workloads:
            (runtime / f"container-{workload.suffix}.stdout").write_text(
                outputs[workload.suffix],
                encoding="utf-8",
            )
        failure_stage = "trace_evidence_validation"
        wait_for_completed_traces(database, len(workloads), args.drain_timeout_seconds)
        trace_by_workload = wait_for_trace_evidence(
            database,
            actrailviewer,
            config,
            workloads,
            args.drain_timeout_seconds,
        )
        failure_stage = "output_and_isolation_validation"
        verify_outputs(outputs, workloads)
        verify_file_outputs(workloads)
        verify_no_cross_trace_llm_actions(
            actrailviewer,
            config,
            trace_by_workload,
            workloads,
        )
        verify_tls_binary_analysis_cache(daemon_log, config)

        succeeded = True
        return 0
    finally:
        for launch in launches:
            terminate_process(launch)
        for container in reversed(containers):
            try:
                container.close()
            except Exception as error:
                print(f"container cleanup failed: {error}", file=sys.stderr)
        if test_image is not None:
            try:
                removed_images = test_image.prune_other_versions()
                if removed_images:
                    print(
                        "removed stale test images: " + ", ".join(removed_images),
                        file=sys.stderr,
                    )
            except Exception as error:
                print(f"image cleanup failed: {error}", file=sys.stderr)
        if daemon is not None:
            terminate_process(daemon)
            if not succeeded:
                daemon_lines = daemon_log.read_text(
                    encoding="utf-8", errors="replace"
                ).splitlines()
                print(
                    "daemon_summary:\n" + "\n".join(daemon_lines[-80:]),
                    file=sys.stderr,
                )
        for provider in provider_processes:
            terminate_process(provider)
            if not succeeded:
                print_process_stderr("provider", provider)
        if not succeeded:
            print(f"failure_stage={failure_stage}", file=sys.stderr)
        if args.keep_runtime or (args.keep_runtime_on_failure and not succeeded):
            print(
                f"multi_container_runtime_preserved={runtime} succeeded={succeeded}",
                file=sys.stderr,
            )
        else:
            shutil.rmtree(runtime, ignore_errors=True)


def prepare_runtime(
    runtime: Path,
    template: Path,
    config: Path,
    actraild: Path,
) -> None:
    for child in ("run", "data", "data/export", "log"):
        (runtime / child).mkdir(parents=True, exist_ok=True)
    rendered = template.read_text(encoding="utf-8").replace(RUNTIME_TOKEN, str(runtime))
    if RUNTIME_TOKEN in rendered:
        raise RuntimeError("operator template still contains an unresolved runtime token")
    patch = runtime / "operator.patch.toml"
    patch.write_text(rendered, encoding="utf-8")
    initialized = subprocess.run(
        [
            str(actraild),
            "--config",
            str(config),
            "init",
            "-f",
            "--patch",
            str(patch),
        ],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if initialized.returncode != 0:
        raise RuntimeError(
            "failed to initialize refreshed operator config: "
            f"stdout={initialized.stdout!r} stderr={initialized.stderr!r}"
        )


def prepare_workload_files(workloads: list[Workload]) -> None:
    for workload in workloads:
        workload.input_path.parent.mkdir(parents=True, exist_ok=True)
        workload.input_path.write_text(workload.input_text, encoding="utf-8")


def prepare_agent_image(
    runtime: Path,
    base_image: str,
    dockerfile: Path,
    force_rebuild: bool,
) -> ContainerImage:
    cache_key = zlib.crc32(b"container-agent-xiaoo-runtime-v2\0")
    cache_key = zlib.crc32(base_image.encode("utf-8"), cache_key)
    cache_key = zlib.crc32(dockerfile.read_bytes(), cache_key)
    version = f"runtime-v2-{cache_key:08x}"
    context = runtime / "image"
    context.mkdir()

    build = ContainerImage(
        image_name="actrail/container-agent-xiaoo",
        version=version,
        dockerfile_path=dockerfile,
        build_context=context,
        build_args={"BASE_IMAGE": base_image},
        force_rebuild=force_rebuild,
    )
    build.ensure()
    return ContainerImage(image_name=build.image_name, version=build.version)


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
            "127.0.0.1",
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
        stderr=log,
        start_new_session=True,
    )


def wait_for_daemon(
    actrailctl: Path,
    config: Path,
    daemon: subprocess.Popen[str],
    daemon_log: Path,
    timeout: float,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if daemon.poll() is not None:
            raise RuntimeError(
                "actraild exited early: "
                + daemon_log.read_text(encoding="utf-8")
            )
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


def start_workload(
    container: TestContainer,
    workload: Workload,
    config: Path,
) -> subprocess.Popen[str]:
    return container.exec(
        [
        "/usr/local/bin/actrailctl",
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
        ],
        environment={
            LOCAL_API_KEY_ENV: LOCAL_API_KEY,
            "ACTRAIL_XIAOO_CONFIG": str(workload.config_path),
            "ACTRAIL_XIAOO_PROMPT": workload.task_prompt,
            "ACTRAIL_TASK_INPUT": str(workload.input_path),
            "ACTRAIL_TASK_OUTPUT": str(workload.output_path),
            "ACTRAIL_TASK_WRITE_MARKER": workload.write_marker,
            "ACTRAIL_TASK_HOLD_SECONDS": str(workload.hold_seconds),
        },
    )


def verify_existing_process_identity_handshake(
    actrailctl: Path,
    config: Path,
    timeout: float,
) -> None:
    root = subprocess.Popen(
        ["/bin/sh", "-c", "sleep 30 & wait"],
        stdin=subprocess.DEVNULL,
        stdout=subprocess.DEVNULL,
        stderr=subprocess.PIPE,
        text=True,
        start_new_session=True,
    )
    try:
        wait_for_process_child(root, timeout)
        run_checked(
            [
                str(actrailctl),
                "--config",
                str(config),
                "track-add",
                "--pid",
                str(root.pid),
                "--name",
                IDENTITY_HANDSHAKE_TRACE_NAME,
            ]
        )
        if os.environ.get("ACTRAIL_REQUIRE_NESTED_PID_IDENTITY") == "1":
            require_cached_identity_pid_separation(root.pid)
        run_checked(
            [
                str(actrailctl),
                "--config",
                str(config),
                "track-remove",
                "--name",
                IDENTITY_HANDSHAKE_TRACE_NAME,
            ]
        )
    finally:
        try:
            os.killpg(root.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        try:
            root.communicate(timeout=5)
        except subprocess.TimeoutExpired:
            try:
                os.killpg(root.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            root.communicate()


def wait_for_process_child(root: subprocess.Popen[str], timeout: float) -> None:
    children_path = Path(f"/proc/{root.pid}/task/{root.pid}/children")
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        if root.poll() is not None:
            _, stderr = root.communicate()
            raise RuntimeError(
                "existing-process identity root exited before attach: "
                f"exit={root.returncode} stderr={stderr}"
            )
        try:
            if children_path.read_text(encoding="utf-8").strip():
                return
        except FileNotFoundError:
            pass
        time.sleep(0.02)
    raise RuntimeError("existing-process identity root did not create its snapshot child")


def verify_nonleader_exec_lifecycle(
    actrailctl: Path,
    config: Path,
    database: Path,
    fixture: Path,
    marker: Path,
    timeout: float,
) -> None:
    completed = subprocess.run(
        [
            str(actrailctl),
            "--config",
            str(config),
            "launch",
            "--name",
            NONLEADER_EXEC_TRACE_NAME,
            "--host-ebpf",
            "required",
            "--seccomp-notify",
            "disabled",
            "--",
            sys.executable,
            str(fixture),
            str(marker),
        ],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if completed.returncode != 0:
        raise RuntimeError(
            "non-leader exec fixture failed: "
            f"exit={completed.returncode} stdout={completed.stdout} stderr={completed.stderr}"
        )
    if marker.read_text(encoding="utf-8").strip() != "NONLEADER_EXEC_COMPLETE":
        raise RuntimeError("non-leader exec fixture did not write its completion marker")

    deadline = time.monotonic() + timeout
    last_row: tuple[int, int] | None = None
    while time.monotonic() < deadline:
        with sqlite3.connect(database) as connection:
            last_row = connection.execute(
                """
                SELECT traces.trace_id, COUNT(events.event_id)
                FROM traces
                LEFT JOIN events ON events.trace_id = traces.trace_id
                  AND events.collector = 'ebpf'
                  AND events.kind = 'process'
                WHERE traces.display_name = ?
                GROUP BY traces.trace_id
                """,
                (NONLEADER_EXEC_TRACE_NAME,),
            ).fetchone()
        if last_row is not None and int(last_row[1]) >= 2:
            return
        time.sleep(0.05)
    raise RuntimeError(
        "non-leader exec did not preserve process lifecycle observation: "
        f"last_row={last_row}"
    )


def require_cached_identity_pid_separation(observer_tgid: int) -> None:
    maps = json.loads(run_checked(["bpftool", "-j", "map", "show"]))
    candidates = [
        item
        for item in maps
        if item.get("name") == "process_identit"
        and item.get("bytes_key") == 4
        and item.get("bytes_value") == 16
    ]
    for candidate in candidates:
        entries = json.loads(
            run_checked(
                ["bpftool", "-j", "map", "dump", "id", str(candidate["id"])]
            )
        )
        for entry in entries:
            key = decode_bpftool_bytes(entry.get("key"))
            value = decode_bpftool_bytes(entry.get("value"))
            if len(key) != 4 or len(value) != 16:
                continue
            cached_observer_tgid = int.from_bytes(value[8:12], sys.byteorder)
            if cached_observer_tgid != observer_tgid:
                continue
            kernel_tgid = int.from_bytes(key, sys.byteorder)
            if kernel_tgid == observer_tgid:
                raise RuntimeError(
                    "nested observer PID unexpectedly equals raw kernel TGID: "
                    f"pid={observer_tgid}"
                )
            print(
                "nested_process_identity="
                f"observer_tgid:{observer_tgid},kernel_tgid:{kernel_tgid}",
                file=sys.stderr,
            )
            return
    raise RuntimeError(
        "nested observer identity was not present in the process identity cache: "
        f"observer_tgid={observer_tgid} maps={[item.get('id') for item in candidates]}"
    )


def decode_bpftool_bytes(value: object) -> bytes:
    if not isinstance(value, list):
        return b""
    decoded = bytearray()
    for item in value:
        if isinstance(item, int):
            decoded.append(item)
        elif isinstance(item, str):
            decoded.append(int(item, 16))
        else:
            return b""
    return bytes(decoded)


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
                      AND display_name IN (?, ?)
                    ORDER BY trace_id
                    """,
                    (f"{PROFILE_NAME}%", *BUSINESS_TRACE_NAMES),
                ).fetchall()
            if len(last_rows) == expected and all(row[1] == "active" for row in last_rows):
                return last_rows
        time.sleep(0.05)
    raise RuntimeError(f"expected traces were not simultaneously active: {last_rows}")


def resolve_docker_seccomp(value: str, repo: Path) -> str:
    if value == "unconfined":
        return "seccomp=unconfined"
    return f"seccomp={require_file(resolve_path(value, repo))}"


def require_distinct_pid_namespaces(pid_namespaces: dict[str, str]) -> None:
    if len(set(pid_namespaces.values())) != len(pid_namespaces):
        raise RuntimeError(
            "workload containers unexpectedly share a PID namespace: "
            f"{pid_namespaces}"
        )


def require_trace_container_isolation(
    trace_rows: list[tuple[int, str, str | None]],
    expected_container_ids: set[str],
) -> None:
    """Compatibility assertion for attribution-specific acceptance cases."""
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
              AND display_name IN (?, ?)
            ORDER BY trace_id
            """,
            (f"{PROFILE_NAME}%", *BUSINESS_TRACE_NAMES),
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
              AND display_name IN (?, ?)
            ORDER BY created_at
            """,
            (f"{PROFILE_NAME}%", *BUSINESS_TRACE_NAMES),
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
                  AND display_name IN (?, ?)
                ORDER BY trace_id
                """,
                (f"{PROFILE_NAME}%", *BUSINESS_TRACE_NAMES),
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
            trace_by_workload = identify_traces_by_response_marker(
                database,
                actrailviewer,
                config,
                workloads,
            )
            for workload in workloads:
                trace_id = trace_by_workload[workload.suffix]
                require_ebpf_evidence(database, trace_id)
                require_llm_response(actrailviewer, config, trace_id)
                require_file_actions(actrailviewer, config, trace_id, workload)
            return trace_by_workload
        except RuntimeError as error:
            last_error = str(error)
            time.sleep(0.2)
    raise RuntimeError(f"trace evidence did not become complete: {last_error}")


def identify_traces_by_response_marker(
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
                  AND display_name IN (?, ?)
                ORDER BY trace_id
                """,
                (f"{PROFILE_NAME}%", *BUSINESS_TRACE_NAMES),
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
                action.get("kind") == "llm.response"
                and workload.response_marker
                in action.get("attributes", {}).get("llm.response.content_text", "")
                for action in actions
            )
        ]
        if len(matches) != 1:
            raise RuntimeError(
                f"response marker {workload.response_marker} matched traces {matches}"
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


def require_llm_response(actrailviewer: Path, config: Path, trace_id: int) -> None:
    actions = load_trace_actions(actrailviewer, config, trace_id)
    responses = [
        action
        for action in actions
        if action.get("kind") == "llm.response"
    ]
    if not responses:
        raise RuntimeError(f"trace-{trace_id} has no llm.response action")
    if any(action.get("status") != "success" for action in responses):
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


def verify_tls_binary_analysis_cache(daemon_log: Path, config: Path) -> None:
    with config.open("rb") as stream:
        capacity = int(
            tomllib.load(stream)["payload"]["tls"][
                "binary_analysis_cache_capacity"
            ]
        )
    peer_lookups: list[tuple[str, str, int]] = []
    seen_peers: set[str] = set()
    for line in daemon_log.read_text(encoding="utf-8").splitlines():
        if "TLS binary analysis cache lookup" not in line:
            continue
        if diagnostic_field(line, "consumer") != "sync":
            continue
        runtime_binary = diagnostic_field(line, "runtime_binary")
        if runtime_binary is None or Path(runtime_binary).name != "xiaoo":
            continue
        probe_binary = diagnostic_field(line, "probe_binary")
        cache = diagnostic_field(line, "cache")
        entries = diagnostic_field(line, "cache_entries")
        if (
            probe_binary is None
            or cache not in {"hit", "miss"}
            or entries is None
        ):
            continue
        peer = re.search(r"^/proc/(\d+)/root/", probe_binary)
        if peer is None or peer.group(1) in seen_peers:
            continue
        seen_peers.add(peer.group(1))
        entry_count = int(entries)
        if entry_count > capacity:
            raise RuntimeError(
                f"TLS binary analysis cache entries {entry_count} exceed {capacity}"
            )
        peer_lookups.append((peer.group(1), cache, entry_count))
    if len(peer_lookups) < 2:
        raise RuntimeError(
            "TLS binary analysis diagnostics lack two xiaoO peer PIDs: "
            f"{peer_lookups}"
        )
    first_peer, second_peer = peer_lookups[:2]
    if second_peer[1] != "hit":
        raise RuntimeError(
            "second xiaoO peer did not reuse binary analysis: "
            f"first={first_peer} second={second_peer}"
        )
    print(
        "tls_binary_analysis_cache "
        f"first_peer={first_peer[0]} first={first_peer[1]} "
        f"second_peer={second_peer[0]} second=hit "
        f"entries={second_peer[2]} capacity={capacity}"
    )


def diagnostic_field(line: str, name: str) -> str | None:
    matched = re.search(rf'(?:^|\s){re.escape(name)}=(?:"([^"]*)"|(\S+))', line)
    if matched is None:
        return None
    return matched.group(1) if matched.group(1) is not None else matched.group(2)


def inspect_pid_namespace(name: str) -> str:
    host_pid = run_checked(
        ["docker", "inspect", "--format", "{{.State.Pid}}", name]
    ).strip()
    return os.readlink(f"/proc/{host_pid}/ns/pid")


def inspect_container_id(name: str) -> str:
    """Compatibility helper for attribution-specific acceptance cases."""
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
