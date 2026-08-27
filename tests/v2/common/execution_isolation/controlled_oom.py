from __future__ import annotations

import ctypes
import os
import subprocess
import sys
import tempfile
import time
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.process import CommandRunner, SubprocessRunner


_ROOT_MARKER = "ACTRAIL_CONTROLLED_HOST_OOM_ROOT"
_OOM_MARKER = "ACTRAIL_HOST_OOM_KILL_OK"
_ROOT_PROCESS_NAME_HEX = b"actrail-root".ljust(16, b"\0").hex()
_REPO = Path(__file__).resolve().parents[4]


def memory_cgroup_problem(
    cgroup_root: Path = Path("/sys/fs/cgroup"),
    *,
    swaps_path: Path = Path("/proc/swaps"),
) -> str | None:
    if (cgroup_root / "cgroup.controllers").is_file():
        parent = cgroup_root
        limit_name = "memory.max"
        swap_limit_name = "memory.swap.max"
        swap_limit_value = "0\n"
    elif (cgroup_root / "memory" / "memory.limit_in_bytes").is_file():
        parent = cgroup_root / "memory"
        limit_name = "memory.limit_in_bytes"
        swap_limit_name = "memory.memsw.limit_in_bytes"
        swap_limit_value = "33554432\n"
    else:
        return "memory cgroup controller is unavailable"
    try:
        active_swap = any(
            line.strip()
            for line in swaps_path.read_text(encoding="ascii").splitlines()[1:]
        )
    except OSError as error:
        return f"host swap state is unavailable: {error}"
    if active_swap and not (parent / swap_limit_name).is_file():
        return "active host swap cannot be bounded by this memory cgroup"
    probe = parent / f"actrail-precheck-{os.getpid()}"
    if probe.exists():
        return f"memory cgroup precheck path already exists: {probe}"
    try:
        probe.mkdir()
        limit = probe / limit_name
        if not limit.is_file():
            return f"memory controller is not enabled below {parent}"
        limit.write_text("33554432\n", encoding="ascii")
        swap_limit = probe / swap_limit_name
        if swap_limit.is_file():
            swap_limit.write_text(swap_limit_value, encoding="ascii")
        elif active_swap:
            return "active host swap cannot be bounded by this memory cgroup"
    except OSError as error:
        return f"memory cgroup is not delegated for the regression: {error}"
    finally:
        try:
            probe.rmdir()
        except OSError:
            pass
    return None


@dataclass(frozen=True)
class MonitoredRootMarker:
    pid: int
    start_time_ticks: int
    executable_name_hex: str

    def as_process(self) -> dict[str, int | str]:
        return {
            "pid": self.pid,
            "start_time_ticks": self.start_time_ticks,
            "executable_name_hex": self.executable_name_hex,
        }


@dataclass(frozen=True)
class ControlledHostOomResult:
    victim_pid: int
    released_at_ms: int
    kernel_oom_kills_before: int
    kernel_oom_kills_after: int
    cgroup_oom_kills_before: int
    cgroup_oom_kills_after: int
    root_marker: MonitoredRootMarker


class ControlledHostOom:
    def __init__(
        self,
        work_dir: Path,
        *,
        runner: CommandRunner | None = None,
    ) -> None:
        self._work_dir = work_dir.resolve()
        self._runner = runner or SubprocessRunner()

    def run_monitored(
        self,
        *,
        root_discovery_settle_seconds: float,
        timeout_seconds: float,
    ) -> ControlledHostOomResult:
        if root_discovery_settle_seconds <= 0:
            raise ValueError(
                "root_discovery_settle_seconds must be positive"
            )
        if timeout_seconds <= root_discovery_settle_seconds:
            raise ValueError(
                "timeout_seconds must exceed root discovery settle time"
            )
        assets = Path(__file__).resolve().parent / "assets"
        with tempfile.TemporaryDirectory(
            prefix="controlled-host-oom-",
            dir=self._work_dir,
        ) as raw_coord_dir:
            coord_dir = Path(raw_coord_dir)
            try:
                result = self._runner.run(
                    (
                        sys.executable,
                        "-m",
                        "tests.v2.common.execution_isolation.controlled_oom",
                        "_run_monitored_root",
                        str(root_discovery_settle_seconds),
                    ),
                    timeout=timeout_seconds,
                    cwd=_REPO,
                    environment={
                        "ACTRAIL_HOST_COORD_DIR": raw_coord_dir,
                        "ACTRAIL_HOST_OOM_SCRIPT": str(
                            assets / "oom-cgroup-trigger.sh"
                        ),
                        "ACTRAIL_HOST_OOM_TRIGGER": str(
                            assets / "oom_trigger.py"
                        ),
                    },
                )
            except BaseException as error:
                cleanup_error = _remove_recorded_cgroup(coord_dir)
                if cleanup_error is not None:
                    raise RuntimeError(cleanup_error) from error
                raise
            cleanup_error = _remove_recorded_cgroup(coord_dir)
            if cleanup_error is not None:
                raise RuntimeError(cleanup_error)
        if result.returncode != 0:
            raise RuntimeError("controlled host OOM failed: " + result.diagnostic)
        root = _fields_for_marker(result.stdout, _ROOT_MARKER)
        oom = _fields_for_marker(result.stdout, _OOM_MARKER)
        parsed = ControlledHostOomResult(
            victim_pid=int(oom["pid"]),
            released_at_ms=int(oom["released_at_ms"]),
            kernel_oom_kills_before=int(oom["before"]),
            kernel_oom_kills_after=int(oom["after"]),
            cgroup_oom_kills_before=int(oom["cgroup_before"]),
            cgroup_oom_kills_after=int(oom["cgroup_after"]),
            root_marker=MonitoredRootMarker(
                pid=int(root["pid"]),
                start_time_ticks=int(root["start_time_ticks"]),
                executable_name_hex=root["executable_name_hex"],
            ),
        )
        if parsed.cgroup_oom_kills_after <= parsed.cgroup_oom_kills_before:
            raise RuntimeError("controlled cgroup OOM kill did not increase")
        if parsed.kernel_oom_kills_after <= parsed.kernel_oom_kills_before:
            raise RuntimeError("kernel OOM kill did not increase")
        if parsed.root_marker.executable_name_hex != _ROOT_PROCESS_NAME_HEX:
            raise RuntimeError("controlled OOM root comm is not actrail-root")
        if parsed.victim_pid <= 0:
            raise RuntimeError("controlled OOM victim PID is invalid")
        return parsed


def _fields_for_marker(output: str, marker: str) -> dict[str, str]:
    lines = [line for line in output.splitlines() if line.startswith(marker + " ")]
    if len(lines) != 1:
        raise RuntimeError(f"expected one {marker} evidence line, found {len(lines)}")
    fields: dict[str, str] = {}
    for item in lines[0].split()[1:]:
        name, separator, value = item.partition("=")
        if not separator or not name or not value or name in fields:
            raise RuntimeError(f"invalid {marker} evidence field: {item!r}")
        fields[name] = value
    return fields


def _remove_recorded_cgroup(coord_dir: Path) -> str | None:
    record = coord_dir / "oom.cgroup"
    try:
        raw = record.read_text(encoding="ascii").strip()
    except FileNotFoundError:
        return None
    except OSError as error:
        return f"controlled OOM cgroup record is unreadable: {error}"
    path = Path(raw).resolve()
    allowed_parents = {
        Path("/sys/fs/cgroup"),
        Path("/sys/fs/cgroup/memory"),
    }
    if (
        path.parent not in allowed_parents
        or not path.name.startswith("actrail-host-oom-")
    ):
        return f"controlled OOM recorded an unsafe cgroup path: {path}"
    deadline = time.monotonic() + 2
    last_error: OSError | None = None
    while path.exists() and time.monotonic() < deadline:
        try:
            path.rmdir()
        except OSError as error:
            last_error = error
            time.sleep(0.05)
        else:
            break
    if path.exists():
        return f"controlled OOM cgroup cleanup failed for {path}: {last_error}"
    return None


def _run_monitored_root(settle_seconds: float) -> int:
    script = _required_environment_path("ACTRAIL_HOST_OOM_SCRIPT", file=True)
    _required_environment_path("ACTRAIL_HOST_OOM_TRIGGER", file=True)
    _required_environment_path("ACTRAIL_HOST_COORD_DIR", file=False)
    _set_process_name()
    root = _current_root_marker()
    print(
        f"{_ROOT_MARKER} pid={root.pid} "
        f"start_time_ticks={root.start_time_ticks} "
        f"executable_name_hex={root.executable_name_hex}",
        flush=True,
    )
    time.sleep(settle_seconds)
    _require_root_process_name()
    completed = subprocess.run(
        ("/bin/sh", str(script)),
        capture_output=True,
        text=True,
        check=False,
    )
    sys.stdout.write(completed.stdout)
    sys.stderr.write(completed.stderr)
    sys.stdout.flush()
    sys.stderr.flush()
    _require_root_process_name()
    return completed.returncode


def _required_environment_path(name: str, *, file: bool) -> Path:
    raw = os.environ.get(name)
    if not raw:
        raise RuntimeError(f"{name} is required")
    path = Path(raw).resolve()
    valid = path.is_file() if file else path.is_dir()
    if not valid:
        kind = "file" if file else "directory"
        raise RuntimeError(f"{name} {kind} is unavailable: {path}")
    return path


def _set_process_name() -> None:
    libc = ctypes.CDLL(None, use_errno=True)
    prctl = getattr(libc, "prctl", None)
    if prctl is None:
        raise RuntimeError("prctl is unavailable")
    if prctl(15, b"actrail-root", 0, 0, 0) != 0:
        error = ctypes.get_errno()
        raise OSError(error, os.strerror(error))


def _current_root_marker() -> MonitoredRootMarker:
    pid = os.getpid()
    stat = Path(f"/proc/{pid}/stat").read_text(encoding="ascii")
    fields = stat.rsplit(") ", 1)
    if len(fields) != 2:
        raise RuntimeError(f"controlled OOM root stat is invalid: {stat!r}")
    tail = fields[1].split()
    if len(tail) <= 19:
        raise RuntimeError(f"controlled OOM root stat is truncated: {stat!r}")
    _require_root_process_name()
    return MonitoredRootMarker(
        pid=pid,
        start_time_ticks=int(tail[19]),
        executable_name_hex=_ROOT_PROCESS_NAME_HEX,
    )


def _require_root_process_name() -> None:
    name = Path(f"/proc/{os.getpid()}/comm").read_bytes().rstrip(b"\n")
    if name != b"actrail-root":
        raise RuntimeError(f"controlled OOM root comm changed: {name!r}")


def _main(argv: list[str]) -> int:
    usage = (
        "usage: controlled_oom.py _run_monitored_root "
        "ROOT_DISCOVERY_SETTLE_SECONDS"
    )
    if len(argv) != 2 or argv[0] != "_run_monitored_root":
        raise SystemExit(usage)
    try:
        settle_seconds = float(argv[1])
    except ValueError as error:
        raise SystemExit(usage) from error
    if settle_seconds <= 0:
        raise SystemExit("ROOT_DISCOVERY_SETTLE_SECONDS must be positive")
    return _run_monitored_root(settle_seconds)


if __name__ == "__main__":
    raise SystemExit(_main(sys.argv[1:]))
