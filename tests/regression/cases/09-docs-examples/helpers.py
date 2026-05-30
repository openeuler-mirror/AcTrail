"""Helpers for docs/examples regression workflows."""

from __future__ import annotations

import os
import re
import select
import subprocess
import threading
import time
from pathlib import Path

from model import FAIL, PASS, CaseResult
from workload_config import read_config, required


WORKLOAD_CONFIG = Path(__file__).resolve().parent / "workload.conf"
TRACE_RE = re.compile(r"trace trace-(\d+) entered Active")


def run_clean(env, example_name: str, workload: dict[str, str]) -> None:
    run_command(
        env,
        [
            env.python,
            str(env.repo_root / "docs/examples/clean.py"),
            "--bin-dir",
            str(env.bin_dir),
            "--example",
            example_name,
        ],
        float(required(workload, "control_timeout_seconds")),
    )


def start_daemon(env, config: Path, workload: dict[str, str]) -> subprocess.Popen[str]:
    daemon = start_process(env, [str(env.release_binary("actraild")), "--config", str(config), "run"])
    ready_output = wait_for_output(daemon, "daemon listening", float(required(workload, "daemon_ready_timeout_seconds")))
    capture_daemon_output(env, daemon, config, ready_output)
    return daemon


def start_process(
    env,
    command: list[str],
    extra_env: dict[str, str] | None = None,
) -> subprocess.Popen:
    process_env = os.environ.copy()
    if extra_env:
        process_env.update(extra_env)
    return subprocess.Popen(
        command,
        cwd=env.repo_root,
        env=process_env,
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
    )


def run_command(env, command: list[str], timeout: float):
    completed = env.run(command, timeout=timeout)
    if completed.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={completed.stdout}\nstderr={completed.stderr}"
        )
    return completed


def track_add(env, config: Path, pid: int, name: str, workload: dict[str, str]) -> int:
    completed = run_command(
        env,
        [
            str(env.release_binary("actrailctl")),
            "--config",
            str(config),
            "track-add",
            "--pid",
            str(pid),
            "--name",
            name,
        ],
        float(required(workload, "control_timeout_seconds")),
    )
    return parse_trace_id(completed.output)


def viewer(env, config: Path, command: str, trace_id: int, *extra: str) -> str:
    workload = read_config(WORKLOAD_CONFIG)
    completed = run_command(
        env,
        [
            str(env.release_binary("actrailviewer")),
            command,
            "--config",
            str(config),
            "--trace-id",
            str(trace_id),
            *extra,
        ],
        float(required(workload, "control_timeout_seconds")),
    )
    return completed.stdout


def wait_for_output(process: subprocess.Popen, fragment: str, timeout: float) -> str:
    if process.stdout is None:
        raise RuntimeError("process stdout is not captured")
    stdout_fd = process.stdout.fileno()
    os.set_blocking(stdout_fd, False)
    deadline = time.monotonic() + timeout
    output = bytearray()
    while time.monotonic() < deadline:
        chunk = read_available(stdout_fd, deadline)
        if chunk:
            output.extend(chunk)
            combined = decode_bytes(bytes(output))
            if fragment in combined:
                return combined
        if process.poll() is not None:
            break
    stderr = read_remaining(process.stderr)
    raise RuntimeError(
        f"process output did not contain {fragment!r}\nstdout={decode_bytes(bytes(output))}\nstderr={stderr}"
    )


def read_available(fd: int, deadline: float) -> bytes:
    remaining = deadline - time.monotonic()
    if remaining <= 0:
        return b""
    readable, _, _ = select.select([fd], [], [], remaining)
    if not readable:
        return b""
    try:
        return os.read(fd, 4096)
    except BlockingIOError:
        return b""


def write_stdin(process: subprocess.Popen, text: str) -> None:
    if process.stdin is None:
        raise RuntimeError("process stdin is not captured")
    process.stdin.write(text.encode())
    process.stdin.flush()


def communicate(process: subprocess.Popen, timeout: float) -> tuple[str, str]:
    try:
        stdout, stderr = process.communicate(timeout=timeout)
    except subprocess.TimeoutExpired as error:
        stop_process(process, read_config(WORKLOAD_CONFIG))
        stdout, stderr = process.communicate()
        raise RuntimeError(
            f"process timed out\nstdout={decode_bytes(stdout)}\nstderr={decode_bytes(stderr)}"
        ) from error
    return decode_bytes(stdout), decode_bytes(stderr)


def stop_process(process: subprocess.Popen | None, workload: dict[str, str]) -> None:
    if process is None:
        return
    if process.poll() is None:
        process.terminate()
        try:
            process.wait(timeout=float(required(workload, "daemon_stop_timeout_seconds")))
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()
    join_output_drains(process)


def record_process_artifacts(result: CaseResult, process: subprocess.Popen | None) -> None:
    if process is None:
        return
    for path in getattr(process, "_actrail_report_paths", ()):
        if path not in result.report_paths:
            result.report_paths.append(path)


def process_artifact_transcript(env, process: subprocess.Popen | None) -> str:
    if process is None:
        return ""
    sections: list[str] = []
    for path_text in getattr(process, "_actrail_report_paths", ()):
        path = Path(path_text)
        sections.append(f"=== artifact {path.name} ===")
        if path.exists():
            sections.append(env.output_tail(path.read_text(encoding="utf-8", errors="replace")))
        else:
            sections.append("missing artifact file")
    return "\n".join(sections)


def capture_daemon_output(env, process: subprocess.Popen, config: Path, ready_output: str) -> None:
    stdout_path, stderr_path = daemon_artifact_paths(env, config)
    stdout_path.parent.mkdir(parents=True, exist_ok=True)
    stdout_path.write_text(ready_output, encoding="utf-8")
    stderr_path.write_text("", encoding="utf-8")
    threads = [
        drain_stream_to_file(process.stdout, stdout_path),
        drain_stream_to_file(process.stderr, stderr_path),
    ]
    setattr(process, "_actrail_report_paths", [str(stdout_path), str(stderr_path)])
    setattr(process, "_actrail_output_threads", threads)


def daemon_artifact_paths(env, config: Path) -> tuple[Path, Path]:
    try:
        raw_name = str(config.relative_to(env.repo_root))
    except ValueError:
        raw_name = str(config)
    stem = re.sub(r"[^A-Za-z0-9_.-]+", "-", raw_name).strip("-")
    return (
        env.output_dir / f"{stem}.actraild.stdout",
        env.output_dir / f"{stem}.actraild.stderr",
    )


def drain_stream_to_file(stream, path: Path) -> threading.Thread:
    thread = threading.Thread(target=drain_stream_to_file_impl, args=(stream, path), daemon=True)
    thread.start()
    return thread


def drain_stream_to_file_impl(stream, path: Path) -> None:
    if stream is None:
        return
    fd = stream.fileno()
    with path.open("ab") as output:
        while True:
            readable, _, _ = select.select([fd], [], [])
            if not readable:
                continue
            try:
                chunk = os.read(fd, 4096)
            except BlockingIOError:
                continue
            if not chunk:
                return
            output.write(chunk)
            output.flush()


def join_output_drains(process: subprocess.Popen) -> None:
    for thread in getattr(process, "_actrail_output_threads", ()):
        thread.join()


def parse_trace_id(output: str) -> int:
    match = TRACE_RE.search(output)
    if not match:
        raise RuntimeError(f"could not parse trace id from output: {output}")
    return int(match.group(1))


def parse_prefixed_int(output: str, prefix: str) -> int:
    for line in output.splitlines():
        if line.startswith(prefix):
            return int(line.removeprefix(prefix))
    raise RuntimeError(f"output did not contain {prefix}")


def add_expected_found_check(
    result: CaseResult,
    name: str,
    expected: str,
    found: str,
    reason: str,
    status: str = PASS,
) -> None:
    result.add_check(name, status, evidence_detail(expected, found), reason)


def evidence_detail(expected: str, found: str) -> str:
    found_text = found if found.startswith("\n") else bullet_evidence([found])
    return f"\n        - expected: {expected}\n        - found: {found_text}"


def bullet_evidence(items: list[str]) -> str:
    return "\n" + "\n".join(f"            - {item}" for item in items)


def evidence_rows(output: str, specs: list[tuple[str, tuple[str, ...]]]) -> str:
    return bullet_evidence([f"{label}: {matching_line(output, fragments)}" for label, fragments in specs])


def event_rows(output: str, specs: list[tuple[str, str, str, tuple[str, ...]]]) -> str:
    return bullet_evidence(
        [
            f"{label}: {event_line(output, domain, operation, detail_fragments)}"
            for label, domain, operation, detail_fragments in specs
        ]
    )


def network_rows(output: str, specs: list[tuple[str, str, tuple[str, ...]]]) -> str:
    return bullet_evidence(
        [
            f"{label}: {network_line(output, operation, detail_fragments)}"
            for label, operation, detail_fragments in specs
        ]
    )


def prefixed_line_evidence(output: str, prefixes: tuple[str, ...]) -> str:
    return bullet_evidence([line_evidence(output, prefix) for prefix in prefixes])


def event_line(output: str, domain: str, operation: str, detail_fragments: tuple[str, ...]) -> str:
    for line in output.splitlines():
        columns = line.split()
        if (
            len(columns) >= 4
            and columns[0].startswith("event-")
            and columns[1] == domain
            and columns[3] == operation
            and all(fragment in line for fragment in detail_fragments)
        ):
            return line
    return f"missing {domain} {operation} row containing {detail_fragments}"


def event_domain_line(output: str, domain: str) -> str | None:
    for line in output.splitlines():
        columns = line.split()
        if len(columns) >= 2 and columns[0].startswith("event-") and columns[1] == domain:
            return line
    return None


def network_line(output: str, operation: str, detail_fragments: tuple[str, ...]) -> str:
    for line in output.splitlines():
        columns = line.split()
        if (
            len(columns) >= 4
            and columns[0].startswith("event-")
            and operation in columns
            and all(fragment in line for fragment in detail_fragments)
        ):
            return line
    return f"missing network {operation} row containing {detail_fragments}"


def matching_line(output: str, fragments: tuple[str, ...]) -> str:
    for line in output.splitlines():
        if all(fragment in line for fragment in fragments):
            return line
    return "missing line containing " + ", ".join(repr(fragment) for fragment in fragments)


def line_evidence(output: str, fragment: str) -> str:
    for line in output.splitlines():
        if fragment in line:
            return line
    return f"missing line containing {fragment!r}"


def first_content_line(output: str) -> str:
    for line in output.splitlines():
        if line.strip():
            return line
    return "empty output"


def read_remaining(stream) -> str:
    if stream is None:
        return ""
    fd = stream.fileno()
    os.set_blocking(fd, False)
    chunks: list[bytes] = []
    try:
        while chunk := os.read(fd, 4096):
            chunks.append(chunk)
    except BlockingIOError:
        pass
    return decode_bytes(b"".join(chunks))


def decode_bytes(data: bytes | str | None) -> str:
    if data is None:
        return ""
    if isinstance(data, str):
        return data
    return data.decode(errors="replace")


def fail_step(env, result: CaseResult, name: str, error: Exception) -> str:
    result.status = FAIL
    result.stderr_tail = env.output_tail(str(error))
    result.add_check(name, FAIL, str(error), "documented example check failed before expected evidence appeared")
    return FAIL
