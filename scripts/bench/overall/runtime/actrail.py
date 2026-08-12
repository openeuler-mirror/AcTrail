"""Actrail lifecycle helpers for the overall benchmark."""

from __future__ import annotations

import subprocess
import time
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime


def prepare_actrail(
    work_dir: Path,
    bin_dir: Path,
    *,
    no_tls_capture: bool = False,
    no_stdio_capture: bool = False,
    no_seccomp: bool = False,
) -> int:
    config = work_dir / "actraild.conf"
    patch = work_dir / "actraild.patch.toml"
    ActrailRuntime.write_isolated_operator_config_patch(
        patch,
        work_dir,
        payload_tls_enabled=False if no_tls_capture else None,
        payload_stdio_enabled=False if no_stdio_capture else None,
        payload_tls_seccomp_syscalls=[] if no_seccomp else None,
        payload_socket_seccomp_syscalls=[] if no_seccomp else None,
    )

    def run(*arguments: str) -> None:
        command = [str(bin_dir / arguments[0]), *arguments[1:]]
        if arguments[0] in ("actraild", "actrailctl"):
            command.insert(1, "--config")
            command.insert(2, str(config))
        completed = subprocess.run(
            command,
            cwd=str(work_dir),
            capture_output=True,
            text=True,
            timeout=120,
            check=False,
        )
        if completed.returncode != 0:
            detail = completed.stderr.strip()[-1000:]
            raise RuntimeError(
                f"actrail command failed ({completed.returncode}): "
                f"{' '.join(command)}\n{detail}"
            )

    run("actraild", "init", "-f", "--patch", str(patch))
    run("actraild", "stop")
    run("actrailctl", "clean")
    run("actraild", "start")
    self_log = work_dir / "log" / "actraild.log"
    deadline = time.monotonic() + 60
    while time.monotonic() < deadline:
        try:
            log_text = self_log.read_text(encoding="utf-8", errors="replace")
        except OSError:
            log_text = ""
        if "host_ebpf_preflight completed" in log_text:
            break
        time.sleep(0.5)
    else:
        raise RuntimeError("actraild did not finish host_ebpf_preflight in time")
    pid_file = work_dir / "run" / "actraild.pid"
    try:
        return int(pid_file.read_text(encoding="utf-8").strip())
    except (OSError, ValueError) as error:
        raise RuntimeError(
            f"cannot read actraild pid file {pid_file}: {error}"
        ) from error


def stop_actrail(work_dir: Path, bin_dir: Path) -> None:
    config = work_dir / "actraild.conf"
    subprocess.run(
        [str(bin_dir / "actraild"), "--config", str(config), "stop"],
        cwd=str(work_dir),
        capture_output=True,
        timeout=60,
        check=False,
    )
    subprocess.run(
        [str(bin_dir / "actrailctl"), "--config", str(config), "clean"],
        cwd=str(work_dir),
        capture_output=True,
        timeout=60,
        check=False,
    )


def storage_footprint_bytes(work_dir: Path) -> int:
    database = work_dir / "data" / "actrail.sqlite"
    if not database.is_file():
        raise RuntimeError(f"actrail database is missing: {database}")
    paths = (database, database.with_name(f"{database.name}-wal"))
    return sum(path.stat().st_size for path in paths if path.exists())
