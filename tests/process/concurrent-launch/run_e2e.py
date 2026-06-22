#!/usr/bin/env python3
"""Run one daemon with concurrent actrailctl launches and verify collection."""

from __future__ import annotations

import argparse
import os
import re
import select
import shutil
import signal
import sqlite3
import subprocess
import sys
import time
from pathlib import Path


TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def parse_args() -> argparse.Namespace:
    test_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(test_dir / "operator.conf"))
    parser.add_argument("--concurrency", type=int, default=100)
    parser.add_argument("--workload", choices=["shell", "xiaoo"], default="shell")
    parser.add_argument("--sleep-sec", type=float, default=8.0)
    parser.add_argument("--ready-timeout-sec", type=float, default=30.0)
    parser.add_argument("--completion-timeout-sec", type=float, default=90.0)
    parser.add_argument("--skip-limit-check", action="store_true")
    parser.add_argument("--xiaoo-bin", default=os.environ.get("XIAOO_BIN", "xiaoo"))
    parser.add_argument("--xiaoo-max-turns", type=int, default=1)
    parser.add_argument(
        "--xiaoo-enable-tools",
        action="store_true",
        help="allow xiaoo tool execution; default keeps real LLM calls but disables tools",
    )
    parser.add_argument("--xiaoo-provider", default=os.environ.get("XIAOO_PROVIDER"))
    parser.add_argument("--xiaoo-model", default=os.environ.get("XIAOO_MODEL"))
    parser.add_argument("--xiaoo-api-base", default=os.environ.get("XIAOO_API_BASE"))
    parser.add_argument(
        "--xiaoo-config",
        default=os.environ.get("XIAOO_CONFIG", str(Path.home() / ".config/xiaoo/config.toml")),
    )
    parser.add_argument(
        "--xiaoo-work-dir",
        default=os.environ.get("XIAOO_WORK_DIR", "/tmp/actrail-concurrent-launch-xiaoo"),
    )
    parser.add_argument(
        "--xiaoo-shared-trace-db",
        action="store_true",
        help="reuse the configured xiaoo trace DB; default isolates each xiaoo process",
    )
    parser.add_argument(
        "--xiaoo-prompt-template",
        default=(
            "Reply with exactly 16 lines. Each line must be {marker}. "
            "Do not add any other text. "
            "This is AcTrail concurrent LLM validation item {index}."
        ),
    )
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    bin_dir = (repo / args.bin_dir).resolve()
    config = Path(args.config).resolve()
    actraild = require_binary(bin_dir, "actraild")
    actrailctl = require_binary(bin_dir, "actrailctl")
    xiaoo = resolve_xiaoo(args.xiaoo_bin) if args.workload == "xiaoo" else None
    values = read_config(config)
    clean_paths(values)
    clean_xiaoo_work_dir(args)
    xiaoo_configs = prepare_xiaoo_configs(args) if args.workload == "xiaoo" else []
    daemon = subprocess.Popen(
        [str(actraild), "--config", str(config), "run"],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    launches: list[subprocess.Popen[str]] = []
    trace_ids: list[int] = []
    markers = [expected_marker(args.workload, index) for index in range(args.concurrency)]
    try:
        wait_for_daemon(daemon, args.ready_timeout_sec)
        for index in range(args.concurrency):
            launches.append(
                spawn_launch(
                    actrailctl,
                    config,
                    xiaoo,
                    args,
                    index,
                    markers[index],
                    xiaoo_configs[index] if xiaoo_configs else None,
                )
            )
        for process in launches:
            trace_ids.append(wait_for_trace(process, args.ready_timeout_sec))
        wait_for_active_traces(Path(values["storage_sqlite_path"]), trace_ids, args.ready_timeout_sec)
        list_output = run_checked([str(actrailctl), "--config", str(config), "list-traces"])
        for trace_id in trace_ids:
            if not any(
                line.startswith(f"trace-{trace_id} ") and clean_list_state(args.workload, line)
                for line in list_output.splitlines()
            ):
                raise RuntimeError(
                    f"list-traces did not show trace-{trace_id} clean: {list_output}"
                )
        if not args.skip_limit_check and args.workload == "shell":
            verify_limit_rejection(
                actrailctl,
                config,
                first_root_pid(Path(values["storage_sqlite_path"]), trace_ids),
            )
        outputs = wait_for_launches(launches, args.completion_timeout_sec)
        wait_for_completed_traces(
            Path(values["storage_sqlite_path"]),
            trace_ids,
            args.completion_timeout_sec,
        )
        verify_launch_outputs(args.workload, outputs, markers)
        verify_payload_markers(Path(values["storage_sqlite_path"]), args.workload, trace_ids, markers)
        print(f"concurrent launch e2e passed traces={','.join(f'trace-{tid}' for tid in trace_ids)}")
        for output in outputs:
            print(output, end="")
        return 0
    finally:
        for process in launches:
            if process.poll() is None:
                process.terminate()
        stop_daemon(daemon)
        print_daemon_stderr(daemon)


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; build with cargo build --release")
    return path


def resolve_xiaoo(value: str) -> Path:
    path = Path(value)
    if path.exists():
        return path.resolve()
    resolved = shutil.which(value)
    if resolved:
        return Path(resolved)
    raise RuntimeError(f"missing xiaoo binary {value}")


def prepare_xiaoo_configs(args: argparse.Namespace) -> list[Path]:
    if args.xiaoo_shared_trace_db:
        config = Path(args.xiaoo_config).expanduser().resolve()
        if not config.exists():
            raise RuntimeError(f"missing xiaoo config {config}")
        return [config for _ in range(args.concurrency)]
    base_config = Path(args.xiaoo_config).expanduser().resolve()
    if not base_config.exists():
        raise RuntimeError(f"missing xiaoo config {base_config}")
    work_dir = Path(args.xiaoo_work_dir)
    configs: list[Path] = []
    for index in range(args.concurrency):
        config_dir = work_dir / f"xiaoo-{index}" / "config"
        data_dir = work_dir / f"xiaoo-{index}" / "data"
        config_dir.mkdir(parents=True, exist_ok=True)
        data_dir.mkdir(parents=True, exist_ok=True)
        config_path = config_dir / "config.toml"
        write_isolated_xiaoo_config(base_config, config_path, data_dir / "trace.db")
        configs.append(config_path)
    return configs


def write_isolated_xiaoo_config(base_config: Path, config_path: Path, db_path: Path) -> None:
    lines = base_config.read_text(encoding="utf-8").splitlines()
    output: list[str] = []
    in_trace = False
    trace_seen = False
    trace_keys_written = False
    for line in lines:
        stripped = line.strip()
        if stripped.startswith("[") and stripped.endswith("]"):
            if in_trace and not trace_keys_written:
                output.extend(isolated_trace_lines(db_path))
                trace_keys_written = True
            in_trace = stripped == "[trace]"
            trace_seen = trace_seen or in_trace
            output.append(line)
            continue
        if in_trace and active_toml_key(stripped) in {"db_path", "storage_backend"}:
            continue
        output.append(line)
    if trace_seen:
        if in_trace and not trace_keys_written:
            output.extend(isolated_trace_lines(db_path))
    else:
        output.extend(["", "[trace]", *isolated_trace_lines(db_path)])
    config_path.write_text("\n".join(output) + "\n", encoding="utf-8")


def active_toml_key(stripped: str) -> str | None:
    if not stripped or stripped.startswith("#") or "=" not in stripped:
        return None
    return stripped.split("=", 1)[0].strip()


def isolated_trace_lines(db_path: Path) -> list[str]:
    return [
        'storage_backend = "moirai-sqlite"',
        f'db_path = "{db_path}"',
    ]


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    section = ""
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]")
            continue
        key, separator, value = line.partition("=")
        if not separator:
            continue
        key = key.strip()
        value = value.strip().strip('"')
        if section.startswith("export.routes.otel-jsonl.") and key == "path":
            key = "export_otel_jsonl_path"
        values.setdefault(key, value)
    return values


def clean_paths(values: dict[str, str]) -> None:
    for key in [
        "socket_path",
        "pid_file",
        "storage_sqlite_path",
        "log_path",
        "payload_tls_sync_event_socket_path",
        "export_otel_jsonl_path",
    ]:
        path = values.get(key)
        if path and Path(path).exists():
            Path(path).unlink()
    export_dir = values.get("export_directory")
    if export_dir and Path(export_dir).exists():
        shutil.rmtree(export_dir)


def clean_xiaoo_work_dir(args: argparse.Namespace) -> None:
    if args.workload == "xiaoo" and not args.xiaoo_shared_trace_db:
        work_dir = Path(args.xiaoo_work_dir)
        if work_dir.exists():
            shutil.rmtree(work_dir)


def wait_for_daemon(process: subprocess.Popen[str], timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    assert process.stdout is not None
    while time.monotonic() < deadline:
        line = read_line(process, deadline)
        if line:
            print(line, end="")
            if "daemon listening" in line:
                return
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"actraild exited early: {stderr}")
    raise RuntimeError("actraild did not become ready")


def spawn_launch(
    actrailctl: Path,
    config: Path,
    xiaoo: Path | None,
    args: argparse.Namespace,
    index: int,
    marker: str,
    xiaoo_config: Path | None,
) -> subprocess.Popen[str]:
    command = workload_command(xiaoo, args, index, marker, xiaoo_config)
    script = (
        f"printf '{marker}_START\\n'; "
        f"sleep {args.sleep_sec}; "
        f"printf '{marker}_END\\n'"
    )
    if args.workload == "shell":
        command = ["sh", "-c", script]
    return subprocess.Popen(
        [
            str(actrailctl),
            "--config",
            str(config),
            "launch",
            "--name",
            f"{args.workload}-{index}",
            "--",
            *command,
        ],
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def workload_command(
    xiaoo: Path | None,
    args: argparse.Namespace,
    index: int,
    marker: str,
    xiaoo_config: Path | None,
) -> list[str]:
    if args.workload == "shell":
        return []
    if xiaoo is None:
        raise RuntimeError("xiaoo workload selected without a xiaoo binary")
    prompt = args.xiaoo_prompt_template.format(index=index, marker=marker)
    command = [str(xiaoo), "run"]
    if xiaoo_config is not None:
        command.extend(["--config", str(xiaoo_config)])
    if not args.xiaoo_enable_tools:
        command.append("--no-tools")
    command.extend(["--max-turns", str(args.xiaoo_max_turns), "-p", prompt])
    if args.xiaoo_provider:
        command.extend(["--provider", args.xiaoo_provider])
    if args.xiaoo_model:
        command.extend(["--model", args.xiaoo_model])
    if args.xiaoo_api_base:
        command.extend(["--api-base", args.xiaoo_api_base])
    return command


def expected_marker(workload: str, index: int) -> str:
    if workload == "xiaoo":
        return f"ACTRAIL_XIAOO_{index}"
    return f"ACTRAIL_CONCURRENT_{index}"


def wait_for_trace(process: subprocess.Popen[str], timeout_sec: float) -> int:
    deadline = time.monotonic() + timeout_sec
    assert process.stdout is not None
    buffered = ""
    while time.monotonic() < deadline:
        line = read_line(process, deadline)
        if line:
            buffered += line
            print(line, end="")
            match = TRACE_RE.search(buffered)
            if match:
                return int(match.group(1))
        if process.poll() is not None:
            stderr = process.stderr.read() if process.stderr else ""
            raise RuntimeError(f"launch exited before trace id stdout={buffered} stderr={stderr}")
    raise RuntimeError(f"timed out waiting for trace id stdout={buffered}")


def read_line(process: subprocess.Popen[str], deadline: float) -> str:
    assert process.stdout is not None
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return ""
    readable, _, _ = select.select([process.stdout], [], [], remaining)
    if readable:
        return process.stdout.readline()
    return ""


def run_checked(command: list[str]) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    print(result.stdout, end="")
    return result.stdout


def verify_limit_rejection(actrailctl: Path, config: Path, pid: int) -> None:
    result = subprocess.run(
        [
            str(actrailctl),
            "--config",
            str(config),
            "track-add",
            "--pid",
            str(pid),
            "--name",
            "concurrent-over-limit",
        ],
        text=True,
        capture_output=True,
        check=False,
    )
    output = f"{result.stdout}\n{result.stderr}"
    if result.returncode == 0 or "active_trace_limit" not in output:
        raise RuntimeError(
            "over-limit track-add was not rejected with active_trace_limit: "
            f"exit={result.returncode} stdout={result.stdout} stderr={result.stderr}"
        )
    print(result.stderr, end="", file=sys.stderr)


def clean_list_state(workload: str, line: str) -> bool:
    if workload == "xiaoo":
        return " Active/Clean" in line or " Completed/Clean" in line
    return " Active/Clean" in line


def first_root_pid(storage: Path, trace_ids: list[int]) -> int:
    placeholders = ",".join("?" for _ in trace_ids)
    with sqlite3.connect(storage) as connection:
        row = connection.execute(
            f"SELECT root_pid FROM traces WHERE trace_id IN ({placeholders}) ORDER BY trace_id LIMIT 1",
            trace_ids,
        ).fetchone()
    if row is None:
        raise RuntimeError(f"missing root pid for traces {trace_ids}")
    return int(row[0])


def wait_for_active_traces(storage: Path, trace_ids: list[int], timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        states = trace_states(storage, trace_ids)
        if len(states) == len(trace_ids) and all(state == "active" for state in states.values()):
            return
        time.sleep(0.1)
    raise RuntimeError(f"traces did not become concurrently active: {trace_states(storage, trace_ids)}")


def wait_for_launches(
    launches: list[subprocess.Popen[str]],
    timeout_sec: float,
) -> list[str]:
    outputs: list[str] = []
    deadline = time.monotonic() + timeout_sec
    for process in launches:
        remaining = max(0.1, deadline - time.monotonic())
        stdout, stderr = process.communicate(timeout=remaining)
        if stderr:
            print(stderr, end="", file=sys.stderr)
        if process.returncode != 0:
            raise RuntimeError(
                f"launch failed exit={process.returncode}\nstdout={stdout}\nstderr={stderr}"
            )
        outputs.append(stdout)
    return outputs


def wait_for_completed_traces(storage: Path, trace_ids: list[int], timeout_sec: float) -> None:
    deadline = time.monotonic() + timeout_sec
    while time.monotonic() < deadline:
        states = trace_states(storage, trace_ids)
        if len(states) == len(trace_ids) and all(
            state == "completed" for state in states.values()
        ):
            return
        time.sleep(0.2)
    raise RuntimeError(f"traces did not complete: {trace_states(storage, trace_ids)}")


def trace_states(storage: Path, trace_ids: list[int]) -> dict[int, str]:
    if not storage.exists():
        return {}
    placeholders = ",".join("?" for _ in trace_ids)
    with sqlite3.connect(storage) as connection:
        rows = connection.execute(
            f"SELECT trace_id, lifecycle_state FROM traces WHERE trace_id IN ({placeholders})",
            trace_ids,
        ).fetchall()
    return {int(trace_id): str(state) for trace_id, state in rows}


def verify_launch_outputs(workload: str, outputs: list[str], markers: list[str]) -> None:
    if workload != "xiaoo":
        return
    for index, marker in enumerate(markers):
        if marker not in outputs[index]:
            raise RuntimeError(f"xiaoo output missing marker {marker}: {outputs[index]}")


def verify_payload_markers(
    storage: Path,
    workload: str,
    trace_ids: list[int],
    markers: list[str],
) -> None:
    with sqlite3.connect(storage) as connection:
        for index, trace_id in enumerate(trace_ids):
            expected = [f"{markers[index]}_START", f"{markers[index]}_END"]
            if workload == "xiaoo":
                expected = [markers[index]]
            for marker in expected:
                count = connection.execute(
                    """
                    SELECT COUNT(*)
                    FROM payload_segments
                    WHERE trace_id = ?
                      AND direction = 'outbound'
                      AND CAST(bytes AS TEXT) LIKE ?
                    """,
                    (trace_id, f"%{marker}%"),
                ).fetchone()[0]
                if count < 1:
                    raise RuntimeError(f"missing stdout marker {marker} for trace-{trace_id}")


def stop_daemon(process: subprocess.Popen[str]) -> None:
    if process.poll() is not None:
        return
    try:
        os.killpg(process.pid, signal.SIGTERM)
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        os.killpg(process.pid, signal.SIGKILL)
        process.wait(timeout=5)


def print_daemon_stderr(process: subprocess.Popen[str]) -> None:
    if process.stderr is None:
        return
    stderr = process.stderr.read()
    if stderr:
        print(stderr, end="", file=sys.stderr)


if __name__ == "__main__":
    raise SystemExit(main())
