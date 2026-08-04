#!/usr/bin/env python3
"""Run two real xiaoO containers and verify all activity-anomaly alerts."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import select
import shlex
import shutil
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from urllib.parse import urlsplit, urlunsplit

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import require_web_time_attribution  # noqa: E402


PLUGIN_ID = "actrail.activity-anomaly"
PLUGIN_INSTANCES = (
    "actrail.activity-anomaly.e2e.primary",
    "actrail.activity-anomaly.e2e.duplicate",
)
PROFILE_NAME = "multi-container-activity-anomaly"
API_KEY_ENV = "ACTRAIL_MULTI_CONTAINER_XIAOO_API_KEY"
API_KEY = "actrail-activity-anomaly-local-key"
EXPECTED_DEFINITIONS = {
    "llm-request-growth",
    "llm-response-growth",
    "command-duration-exceeded",
}
COMMAND_THRESHOLD_MS = 500
MAX_LIVE_COMMAND_DURATION_MS = 2_000


def load_base_module(repo: Path):
    path = repo / "tests/agent-trace/multi-container-xiaoo/run_e2e.py"
    spec = importlib.util.spec_from_file_location("actrail_multi_container_base", path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"cannot load E2E support module {path}")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    module.PROFILE_NAME = PROFILE_NAME
    return module


def parse_args() -> argparse.Namespace:
    case_dir = Path(__file__).resolve().parent
    repo = case_dir.parents[2]
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
    parser.add_argument(
        "--operator-template",
        default=str(repo / "tests/agent-trace/multi-container-xiaoo/operator.conf"),
    )
    parser.add_argument(
        "--seccomp-profile",
        default=str(repo / "deploy/container-auto/seccomp/actrail-notify.json"),
    )
    parser.add_argument("--ready-timeout-seconds", type=float, default=30.0)
    parser.add_argument("--launch-timeout-seconds", type=float, default=180.0)
    parser.add_argument("--drain-timeout-seconds", type=float, default=45.0)
    parser.add_argument("--runtime-root", default="/tmp")
    parser.add_argument("--keep-runtime", action="store_true")
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    case_dir = Path(__file__).resolve().parent
    repo = case_dir.parents[2]
    base = load_base_module(repo)
    base.require_root()
    base.require_command("docker")

    bin_dir = base.resolve_path(args.bin_dir, repo)
    actraild = base.require_executable(bin_dir / "actraild")
    actrailctl = base.require_executable(bin_dir / "actrailctl")
    actrailviewer = base.require_executable(bin_dir / "actrailviewer")
    actrailweb = base.require_executable(bin_dir / "actrailweb")
    tls_runtime = base.require_file(bin_dir / "libactrail_tls_payload_probe_sync.so")
    xiaoo = base.require_executable(base.resolve_path(args.xiaoo_bin, repo))
    operator_template = base.require_file(base.resolve_path(args.operator_template, repo))
    workload_script = base.require_file(case_dir / "workload.sh")
    provider_script = base.require_file(case_dir / "tool_provider.py")
    plugin_config = base.require_file(case_dir / "activity-anomaly.e2e.config.json")
    plugin_dir = repo / "examples/plugins/wit-component/activity-anomaly"
    plugin_manifest_source = base.require_file(plugin_dir / "activity-anomaly.plugin.toml")
    plugin_artifact = base.require_file(
        plugin_dir
        / "target/wasm32-wasip2/release/actrail_activity_anomaly_plugin.wasm"
    )
    long_command_source = base.require_file(plugin_dir / "long-running-command.sh")
    docker_seccomp = base.resolve_docker_seccomp(args.seccomp_profile, repo)
    runtime_root = base.resolve_path(args.runtime_root, repo)
    if not runtime_root.is_dir():
        raise RuntimeError(f"activity E2E runtime root is not a directory: {runtime_root}")

    runtime = Path(
        tempfile.mkdtemp(
            prefix="actrail-multi-container-activity.",
            dir=runtime_root,
        )
    )
    run_id = f"{int(time.time())}-{os.getpid()}"
    label = f"io.actrail.activity-anomaly-e2e={run_id}"
    workloads = [
        base.Workload(
            suffix="a",
            trace_name="activity-container-a",
            request_marker="ACTRAIL_ACTIVITY_REQUEST_A",
            response_marker="ACTRAIL_ACTIVITY_RESPONSE_A_COMPLETE",
            task_prompt="Run the requested test command, then finish task A.",
            write_marker="unused-a",
            container_name=f"actrail-activity-a-{run_id}",
            config_path=runtime / "xiaoo-a.toml",
            input_path=runtime / "unused-a.in",
            output_path=runtime / "unused-a.out",
            input_text="",
            hold_seconds=0,
        ),
        base.Workload(
            suffix="b",
            trace_name="activity-container-b",
            request_marker="ACTRAIL_ACTIVITY_REQUEST_B",
            response_marker="ACTRAIL_ACTIVITY_RESPONSE_B_COMPLETE",
            task_prompt="Run the requested test command, then finish task B.",
            write_marker="unused-b",
            container_name=f"actrail-activity-b-{run_id}",
            config_path=runtime / "xiaoo-b.toml",
            input_path=runtime / "unused-b.in",
            output_path=runtime / "unused-b.out",
            input_text="",
            hold_seconds=0,
        ),
    ]
    hold_gates = {
        workload.suffix: (
            runtime / f"provider-{workload.suffix}.hold-ready",
            runtime / f"provider-{workload.suffix}.hold-release",
        )
        for workload in workloads
    }
    config = runtime / "operator.conf"
    database = runtime / "data/actrail.sqlite"
    daemon_log = runtime / "log/daemon.stdout"
    providers: list[subprocess.Popen[str]] = []
    launches: list[subprocess.Popen[str]] = []
    daemon: subprocess.Popen[str] | None = None
    succeeded = False

    try:
        prepare_runtime(runtime, operator_template, config)
        long_command_script = runtime / "long-running-command.sh"
        shutil.copy2(long_command_source, long_command_script)
        long_command = (
            f"/bin/bash {shlex.quote(str(long_command_script))} 1 4"
        )
        plugin_manifest = prepare_plugin_package(
            runtime,
            plugin_dir,
            plugin_manifest_source,
            plugin_artifact,
        )
        for workload in workloads:
            hold_ready, hold_release = hold_gates[workload.suffix]
            provider, provider_url = start_provider(
                provider_script,
                workload.response_marker,
                hold_ready,
                hold_release,
                long_command,
                args.ready_timeout_seconds,
                repo,
            )
            providers.append(provider)
            write_xiaoo_config(
                workload.config_path,
                rewrite_provider_host(provider_url, "127.0.0.1"),
            )

        daemon = base.start_daemon(actraild, config, daemon_log)
        base.wait_for_daemon(
            actrailctl,
            config,
            daemon,
            args.ready_timeout_seconds,
        )
        for plugin_instance in PLUGIN_INSTANCES:
            load_plugin(
                actraild,
                config,
                plugin_manifest,
                plugin_config,
                plugin_artifact,
                plugin_instance,
            )

        for workload in workloads:
            launches.append(
                base.start_container(
                    args,
                    workload,
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
        trace_rows = base.wait_for_active_traces(
            database,
            len(workloads),
            args.ready_timeout_seconds,
            launches,
            workloads,
        )
        container_ids = {
            workload.container_name: base.inspect_container_id(workload.container_name)
            for workload in workloads
        }
        base.require_trace_container_isolation(trace_rows, set(container_ids.values()))

        wait_for_hold_commands(
            [ready for ready, _release in hold_gates.values()],
            launches,
            workloads,
            args.launch_timeout_seconds,
        )
        live_alert_rows = wait_for_running_command_alerts(
            database,
            len(workloads),
            args.drain_timeout_seconds,
        )
        require_live_alert_delivery(database, launches, workloads)
        verify_running_command_alerts(live_alert_rows, len(workloads))
        print(
            "activity_anomaly_live_delivery "
            f"agents_running={len(launches)} alerts={len(live_alert_rows)} "
            "trace_states=active"
        )
        release_hold_commands([release for _ready, release in hold_gates.values()])
        outputs = base.wait_for_launches(
            launches,
            workloads,
            args.launch_timeout_seconds,
        )
        verify_agent_outputs(outputs, workloads)
        base.wait_for_completed_traces(
            database,
            len(workloads),
            args.drain_timeout_seconds,
        )
        trace_rows_by_name = trace_rows_by_display_name(database)
        verify_trace_identity(trace_rows_by_name, workloads, container_ids)
        alert_rows = wait_for_stable_alerts(
            database,
            len(workloads) * len(EXPECTED_DEFINITIONS),
            args.drain_timeout_seconds,
        )
        actions_by_trace = {
            trace_id: base.load_trace_actions(actrailviewer, config, trace_id)
            for trace_id, _container_id in trace_rows_by_name.values()
        }
        attribution_by_trace = {
            trace_id: require_web_time_attribution(
                actrailweb,
                config,
                trace_id,
                args.ready_timeout_seconds,
                0.25,
                require_tool=True,
            )
            for trace_id, _container_id in trace_rows_by_name.values()
        }
        verify_alerts(
            alert_rows,
            trace_rows_by_name,
            workloads,
            container_ids,
            actions_by_trace,
        )
        for plugin_instance in PLUGIN_INSTANCES:
            status = base.run_checked(
                [
                    str(actraild),
                    "--config",
                    str(config),
                    "plugin",
                    "status",
                    "--instance",
                    plugin_instance,
                ]
            )
            if "last_error=none" not in status:
                raise RuntimeError(
                    f"plugin {plugin_instance} reported an error after analysis:\n{status}"
                )

        for workload in workloads:
            trace_id, container_id = trace_rows_by_name[workload.trace_name]
            print(
                "activity_anomaly_trace "
                f"container={workload.container_name} "
                f"container_id={container_id} "
                f"trace=trace-{trace_id} "
                f"definitions={','.join(sorted(EXPECTED_DEFINITIONS))}"
            )
            print(
                "activity_anomaly_time_attribution "
                f"trace=trace-{trace_id} "
                f"model_nanos={attribution_by_trace[trace_id]['model_nanos']} "
                f"agent_nanos={attribution_by_trace[trace_id]['agent_nanos']} "
                f"tools={attribution_by_trace[trace_id]['named_tool_count']}"
            )
        print("multi-container real-agent activity anomaly E2E complete")
        succeeded = True
        return 0
    finally:
        for launch in launches:
            base.terminate_process(launch)
        base.remove_owned_containers(label)
        if daemon is not None:
            base.terminate_process(daemon)
            base.print_process_stderr("daemon", daemon)
        for provider in providers:
            base.terminate_process(provider)
            base.print_process_stderr("activity_provider", provider)
        if args.keep_runtime:
            print(
                f"activity_anomaly_runtime_preserved={runtime} succeeded={succeeded}",
                file=sys.stderr,
            )
        else:
            shutil.rmtree(runtime, ignore_errors=True)


def prepare_runtime(runtime: Path, template: Path, config: Path) -> None:
    for child in ("run", "data", "data/export", "log"):
        (runtime / child).mkdir(parents=True, exist_ok=True)
    rendered = template.read_text(encoding="utf-8")
    rendered = rendered.replace("@RUNTIME_DIR@", str(runtime))
    rendered = rendered.replace(
        'profile_name = "multi-container-xiaoo"',
        f'profile_name = "{PROFILE_NAME}"',
    )
    rendered = rendered.replace(
        "[process_seccomp]\nenabled = false",
        "[process_seccomp]\nenabled = true",
    )
    if "@RUNTIME_DIR@" in rendered:
        raise RuntimeError("operator template has an unresolved runtime token")
    if f'profile_name = "{PROFILE_NAME}"' not in rendered:
        raise RuntimeError("operator template profile replacement failed")
    if "[process_seccomp]\nenabled = true" not in rendered:
        raise RuntimeError("activity E2E process argv capture was not enabled")
    config.write_text(rendered, encoding="utf-8")


def prepare_plugin_package(
    runtime: Path,
    source_dir: Path,
    manifest_source: Path,
    artifact_source: Path,
) -> Path:
    package_dir = runtime / "plugins/activity-anomaly"
    package_dir.mkdir(parents=True, exist_ok=True)
    assets = [
        manifest_source.name,
        "activity-anomaly.config.v1.schema.json",
        "llm-growth.payload.v1.schema.json",
        "command-duration.payload.v1.schema.json",
    ]
    for asset in assets:
        shutil.copy2(source_dir / asset, package_dir / asset)
    shutil.copy2(
        artifact_source,
        package_dir / "actrail_activity_anomaly_plugin.wasm",
    )
    return package_dir / manifest_source.name


def start_provider(
    script: Path,
    response_marker: str,
    hold_ready: Path,
    hold_release: Path,
    long_command: str,
    timeout: float,
    cwd: Path,
) -> tuple[subprocess.Popen[str], str]:
    process = subprocess.Popen(
        [
            sys.executable,
            str(script),
            "--response-marker",
            response_marker,
            "--sleep-seconds",
            "2",
            "--hold-ready",
            str(hold_ready),
            "--hold-release",
            str(hold_release),
            "--long-command",
            long_command,
        ],
        cwd=cwd,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )
    if process.stdout is None:
        raise RuntimeError("activity provider stdout is unavailable")
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        remaining = max(0.0, deadline - time.monotonic())
        readable, _, _ = select.select([process.stdout], [], [], remaining)
        if not readable:
            break
        line = process.stdout.readline()
        if line.startswith("provider_base_url="):
            return process, line.split("=", 1)[1].strip()
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr is not None else ""
            raise RuntimeError(f"activity provider exited early: {stderr}")
    raise RuntimeError("activity provider did not report its listen URL")


def wait_for_hold_commands(
    ready_paths: list[Path],
    launches: list[subprocess.Popen[str]],
    workloads: list,
    timeout: float,
) -> None:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        for launch, workload in zip(launches, workloads):
            if launch.poll() is not None:
                stdout, stderr = launch.communicate()
                raise RuntimeError(
                    f"{workload.container_name} exited before the live-alert hold command "
                    f"exit={launch.returncode} stdout={stdout} stderr={stderr}"
                )
        if all(path.is_file() for path in ready_paths):
            return
        time.sleep(0.05)
    missing = [str(path) for path in ready_paths if not path.is_file()]
    raise RuntimeError(f"real agents did not start their hold commands: {missing}")


def release_hold_commands(release_paths: list[Path]) -> None:
    for path in release_paths:
        path.write_text("release\n", encoding="utf-8")


def require_live_alert_delivery(
    database: Path,
    launches: list[subprocess.Popen[str]],
    workloads: list,
) -> None:
    stopped = [
        workload.container_name
        for launch, workload in zip(launches, workloads)
        if launch.poll() is not None
    ]
    if stopped:
        raise RuntimeError(f"agents stopped before live alerts were verified: {stopped}")
    with sqlite3.connect(database) as connection:
        rows = connection.execute(
            """
            SELECT display_name, lifecycle_state
            FROM traces
            WHERE profile_name LIKE ?
            ORDER BY trace_id
            """,
            (f"{PROFILE_NAME}%",),
        ).fetchall()
    expected_names = {workload.trace_name for workload in workloads}
    actual_names = {str(name) for name, _state in rows}
    if actual_names != expected_names or any(state != "active" for _name, state in rows):
        raise RuntimeError(f"alerts were not delivered while all traces were active: {rows}")


def rewrite_provider_host(url: str, host: str) -> str:
    parsed = urlsplit(url)
    if parsed.port is None:
        raise RuntimeError(f"provider URL has no port: {url}")
    return urlunsplit((parsed.scheme, f"{host}:{parsed.port}", parsed.path, "", ""))


def write_xiaoo_config(path: Path, provider_url: str) -> None:
    path.write_text(
        "\n".join(
            [
                "[llm]",
                'provider = "deepseek"',
                'model = "deepseek-chat"',
                f'api_key_env = "{API_KEY_ENV}"',
                f'api_base = "{provider_url}"',
                "max_tokens = 128",
                "context_window = 32768",
                'reasoning_effort = "off"',
                "",
            ]
        ),
        encoding="utf-8",
    )


def load_plugin(
    actraild: Path,
    config: Path,
    manifest: Path,
    plugin_config: Path,
    artifact: Path,
    plugin_instance: str,
) -> None:
    if not artifact.is_file():
        raise RuntimeError(f"missing activity plugin artifact {artifact}")
    result = subprocess.run(
        [
            str(actraild),
            "--config",
            str(config),
            "plugin",
            "load",
            "--manifest",
            str(manifest),
            "--plugin-config",
            str(plugin_config),
            "--grant",
            "trace-activity-read",
            "--grant",
            "alert-write",
            "--instance",
            plugin_instance,
        ],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    if result.returncode != 0:
        raise RuntimeError(
            f"activity plugin load failed exit={result.returncode}: "
            f"{result.stdout}\n{result.stderr}"
        )
    if plugin_instance not in result.stdout:
        raise RuntimeError(f"plugin load output omitted instance: {result.stdout}")


def verify_agent_outputs(outputs: dict[str, str], workloads: list) -> None:
    for workload in workloads:
        if workload.response_marker not in outputs[workload.suffix]:
            raise RuntimeError(
                f"real agent {workload.suffix} output lacks final marker: "
                f"{outputs[workload.suffix]}"
            )


def trace_rows_by_display_name(database: Path) -> dict[str, tuple[int, str]]:
    with sqlite3.connect(database) as connection:
        rows = connection.execute(
            """
            SELECT trace_id, display_name, root_container_id
            FROM traces
            WHERE profile_name LIKE ?
            ORDER BY trace_id
            """,
            (f"{PROFILE_NAME}%",),
        ).fetchall()
    result: dict[str, tuple[int, str]] = {}
    for trace_id, display_name, container_id in rows:
        if container_id is None:
            raise RuntimeError(f"trace-{trace_id} has no container identity")
        result[str(display_name)] = (int(trace_id), str(container_id))
    return result


def verify_trace_identity(
    rows: dict[str, tuple[int, str]],
    workloads: list,
    container_ids: dict[str, str],
) -> None:
    expected_names = {workload.trace_name for workload in workloads}
    if set(rows) != expected_names:
        raise RuntimeError(f"trace display names mismatch: {rows}")
    trace_ids = set()
    for workload in workloads:
        trace_id, container_id = rows[workload.trace_name]
        trace_ids.add(trace_id)
        expected_container = container_ids[workload.container_name]
        if container_id != expected_container:
            raise RuntimeError(
                f"trace-{trace_id} container mismatch "
                f"actual={container_id} expected={expected_container}"
            )
    if len(trace_ids) != len(workloads):
        raise RuntimeError(f"workloads were not isolated into separate traces: {rows}")


def wait_for_running_command_alerts(
    database: Path,
    expected: int,
    timeout: float,
) -> list[tuple]:
    deadline = time.monotonic() + timeout
    rows: list[tuple] = []
    while time.monotonic() < deadline:
        rows = activity_alert_rows(database)
        command_rows = [
            row for row in rows if row[1] == "command-duration-exceeded"
        ]
        if len(command_rows) > expected:
            raise RuntimeError(
                f"command alerts were duplicated during live delivery: {command_rows}"
            )
        if len(command_rows) == expected:
            return rows
        time.sleep(0.1)
    raise RuntimeError(
        f"expected {expected} running-command alerts, found {rows}"
    )


def verify_running_command_alerts(rows: list[tuple], expected: int) -> None:
    command_payloads = [
        json.loads(payload_json)
        for _trace_id, definition, _kind, payload_json in rows
        if definition == "command-duration-exceeded"
    ]
    if len(command_payloads) != expected:
        raise RuntimeError(
            f"expected {expected} running-command alerts, found {command_payloads}"
        )
    for payload in command_payloads:
        if payload.get("maximum_duration_ms") != COMMAND_THRESHOLD_MS:
            raise RuntimeError(f"command threshold mismatch: {payload}")
        findings = payload.get("findings")
        if not isinstance(findings, list) or not findings:
            raise RuntimeError(f"running-command alert has no findings: {payload}")
        for finding in findings:
            if finding.get("status") != "in_progress":
                raise RuntimeError(
                    f"long command was not reported while running: {finding}"
                )
            if finding.get("ended_at_ms") is not None:
                raise RuntimeError(
                    f"running command unexpectedly has an end time: {finding}"
                )
            duration_ms = int(finding.get("duration_ms", 0))
            if not (
                COMMAND_THRESHOLD_MS
                < duration_ms
                <= MAX_LIVE_COMMAND_DURATION_MS
            ):
                raise RuntimeError(
                    f"running command was not reported promptly after threshold: {finding}"
                )
            observed_at_ms = int(finding.get("observed_at_ms", 0))
            started_at_ms = int(finding.get("started_at_ms", 0))
            if observed_at_ms - started_at_ms != duration_ms:
                raise RuntimeError(
                    f"running command duration and observation time disagree: {finding}"
                )
            if not finding.get("executable") or not finding.get("agent_action_id"):
                raise RuntimeError(
                    f"running command omitted executable or Agent attribution: {finding}"
                )


def wait_for_stable_alerts(database: Path, expected: int, timeout: float) -> list[tuple]:
    deadline = time.monotonic() + timeout
    stable_since: float | None = None
    rows: list[tuple] = []
    while time.monotonic() < deadline:
        rows = activity_alert_rows(database)
        if len(rows) > expected:
            raise RuntimeError(
                f"activity alerts duplicated after terminal fallback: {rows}"
            )
        if len(rows) == expected:
            stable_since = stable_since or time.monotonic()
            if time.monotonic() - stable_since >= 1.5:
                return rows
        else:
            stable_since = None
        time.sleep(0.1)
    raise RuntimeError(f"expected {expected} stable activity alerts, found {rows}")


def activity_alert_rows(database: Path) -> list[tuple]:
    with sqlite3.connect(database) as connection:
        return connection.execute(
            """
            SELECT a.trace_id, d.definition_key, d.kind, a.payload_json
            FROM alerts a
            JOIN alert_definitions d
              ON d.alert_definition_id = a.alert_definition_id
            WHERE d.producer_plugin_id = ?
            ORDER BY a.trace_id, d.definition_key
            """,
            (PLUGIN_ID,),
        ).fetchall()


def verify_alerts(
    rows: list[tuple],
    trace_rows: dict[str, tuple[int, str]],
    workloads: list,
    container_ids: dict[str, str],
    actions_by_trace: dict[int, list[dict[str, object]]],
) -> None:
    expected_by_trace = {
        trace_rows[workload.trace_name][0]: (
            workload,
            container_ids[workload.container_name],
        )
        for workload in workloads
    }
    seen: dict[int, set[str]] = {trace_id: set() for trace_id in expected_by_trace}
    root_process_ids: set[str] = set()
    for raw_trace_id, definition, kind, payload_json in rows:
        trace_id = int(raw_trace_id)
        if trace_id not in expected_by_trace:
            raise RuntimeError(f"alert escaped expected traces: trace-{trace_id}")
        workload, expected_container = expected_by_trace[trace_id]
        payload = json.loads(payload_json)
        if not isinstance(payload, dict):
            raise RuntimeError(f"{definition} payload is not an object")
        if payload.get("root_container_id") != expected_container:
            raise RuntimeError(
                f"trace-{trace_id} {definition} container attribution mismatch: {payload}"
            )
        if payload.get("display_name") != workload.trace_name:
            raise RuntimeError(
                f"trace-{trace_id} {definition} display name mismatch: {payload}"
            )
        root_process_id = payload.get("root_process_id")
        if not isinstance(root_process_id, str) or not root_process_id:
            raise RuntimeError(f"trace-{trace_id} has invalid root process identity")
        root_process_ids.add(root_process_id)
        findings = payload.get("findings")
        if not isinstance(findings, list) or not findings:
            raise RuntimeError(f"trace-{trace_id} {definition} has no findings")
        if payload.get("truncated_count") != 0:
            raise RuntimeError(f"trace-{trace_id} {definition} was unexpectedly truncated")
        if definition in {"llm-request-growth", "llm-response-growth"}:
            verify_llm_alert(
                definition,
                kind,
                findings,
                trace_id,
                actions_by_trace[trace_id],
            )
        elif definition == "command-duration-exceeded":
            verify_command_alert(kind, findings, trace_id, actions_by_trace[trace_id])
        else:
            raise RuntimeError(f"unexpected activity alert definition {definition}")
        if definition in seen[trace_id]:
            raise RuntimeError(f"duplicate {definition} alert for trace-{trace_id}")
        seen[trace_id].add(str(definition))
    for trace_id, definitions in seen.items():
        if definitions != EXPECTED_DEFINITIONS:
            raise RuntimeError(
                f"trace-{trace_id} alert definitions mismatch: {definitions}"
            )
    if len(root_process_ids) != len(workloads):
        raise RuntimeError(
            f"container traces shared a root process identity: {root_process_ids}"
        )


def verify_llm_alert(
    definition: str,
    kind: str,
    findings: list,
    trace_id: int,
    actions: list[dict[str, object]],
) -> None:
    expected_kind = (
        "llm.request.growth"
        if definition == "llm-request-growth"
        else "llm.response.growth"
    )
    expected_action_kind = (
        "llm.request" if definition == "llm-request-growth" else "llm.response"
    )
    if kind != expected_kind:
        raise RuntimeError(f"{definition} kind mismatch: {kind}")
    for finding in findings:
        if finding.get("reason") != "hard-limit":
            raise RuntimeError(f"{definition} did not exercise hard limit: {finding}")
        if int(finding.get("observed_bytes", 0)) < 1:
            raise RuntimeError(f"{definition} observed no real payload bytes: {finding}")
        require_action_kind(
            actions,
            trace_id,
            str(finding.get("action_id")),
            expected_action_kind,
        )


def verify_command_alert(
    kind: str,
    findings: list,
    trace_id: int,
    actions: list[dict[str, object]],
) -> None:
    if kind != "command.duration.exceeded":
        raise RuntimeError(f"command alert kind mismatch: {kind}")
    if not any(
        int(finding.get("duration_ms", 0)) > COMMAND_THRESHOLD_MS
        and finding.get("agent_action_id")
        for finding in findings
    ):
        raise RuntimeError(
            f"trace-{trace_id} command alert has no real long agent command line: {findings}"
        )
    for finding in findings:
        if not finding.get("executable"):
            raise RuntimeError(
                f"trace-{trace_id} command alert omitted executable: {finding}"
            )
        require_action_kind(
            actions,
            trace_id,
            str(finding.get("action_id")),
            "command.invocation",
        )


def require_action_kind(
    actions: list[dict[str, object]],
    trace_id: int,
    action_id: str,
    expected_kind: str,
) -> None:
    matches = [
        action
        for action in actions
        if action.get("action_id") == action_id
        and action.get("kind") == expected_kind
    ]
    if len(matches) != 1:
        raise RuntimeError(
            f"trace-{trace_id} action {action_id} kind mismatch: {matches}"
        )


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except Exception as error:
        print(f"multi-container activity anomaly E2E failed: {error}", file=sys.stderr)
        raise SystemExit(1)
