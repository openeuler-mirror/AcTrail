"""Own one isolated daemon and account for completed trace finalization."""

from __future__ import annotations

import re
import os
import sqlite3
import subprocess
import time
import tomllib
from pathlib import Path

from scripts.bench.overall.runtime import prepare_actrail
from scripts.bench.payload.measurement import DaemonCpu


class CollectionRuntime:
    def __init__(self, work_dir: Path, bin_dir: Path, patch: Path, settings: dict):
        self.work_dir, self.bin_dir, self.patch = work_dir, bin_dir, patch
        self.settings = settings
        self.config = work_dir / "actraild.conf"
        self.database = work_dir / "data/actrail.sqlite"
        self.log = None
        self.cpu = None

    @staticmethod
    def running_daemons() -> list[dict]:
        running = []
        for entry in Path("/proc").iterdir():
            if not entry.name.isdigit():
                continue
            try:
                if (entry / "comm").read_text().strip() == "actraild":
                    fields = (entry / "stat").read_text().rsplit(")", 1)[1].split()
                    if fields[0] != "Z":
                        running.append(dict(pid=int(entry.name), start_ticks=fields[19],
                                            executable=str((entry / "exe").resolve(strict=True))))
            except (OSError, ProcessLookupError):
                continue
        return running

    @staticmethod
    def require_stopped() -> None:
        running = CollectionRuntime.running_daemons()
        if running:
            raise RuntimeError("validation requires an isolated daemon environment; running PIDs: "
                               + ", ".join(str(row['pid']) for row in running))

    def start(self) -> None:
        self.work_dir.mkdir()
        try:
            pid = prepare_actrail(self.work_dir, self.bin_dir, config_patch=self.patch)
        except BaseException:
            # Startup may spawn the daemon before its readiness check fails.
            # Recover only the owner of this newly created isolated directory.
            try:
                pid = int((self.work_dir / "run/actraild.pid").read_text())
                owner = DaemonCpu(pid)
                process = Path(f"/proc/{pid}")
                arguments = (process / "cmdline").read_bytes().split(b"\0")
                configured = any(
                    key == b"--config" and value == os.fsencode(self.config)
                    for key, value in zip(arguments, arguments[1:]))
                if configured and (process / "exe").resolve(strict=True) == (self.bin_dir / "actraild").resolve():
                    owner.read_ms()
                    self.cpu = owner
            except (OSError, ValueError, RuntimeError):
                pass
            raise
        if Path(f"/proc/{pid}/exe").resolve(strict=True) != (self.bin_dir / "actraild").resolve():
            raise RuntimeError("isolated daemon executable does not match benchmark binary")
        self.cpu = DaemonCpu(pid)
        self.log = (self.work_dir / "log/actraild.log").open(errors="replace")

    def stop(self) -> None:
        if self.log:
            self.log.close()
        owned = self.cpu is not None and any(
            row['pid'] == self.cpu.pid and row['start_ticks'] == self.cpu.start_time
            for row in self.running_daemons())
        if owned and self.config.exists():
            if int((self.work_dir / 'run/actraild.pid').read_text()) != self.cpu.pid:
                raise RuntimeError("isolated daemon pid file no longer matches benchmark owner")
            config = tomllib.loads(self.config.read_text())
            shutdown_timeout = config["supervision"]["shutdown_wait_ms"] / 1000 + 5
            subprocess.run(
                [str(self.bin_dir / "actraild"), "--config", str(self.config), "stop"],
                capture_output=True, text=True, timeout=shutdown_timeout, check=True,
            )
        if self.cpu and any(row['pid'] == self.cpu.pid and row['start_ticks'] == self.cpu.start_time
                            for row in self.running_daemons()):
            raise RuntimeError("benchmark daemon did not stop")

    def launch(self, command: list[str]) -> list[str]:
        return [str(self.bin_dir / "actrailctl"), "--config", str(self.config), "launch", "--", *command]

    def query(self, sql: str, parameters: tuple = ()) -> list:
        with sqlite3.connect(f"file:{self.database}?mode=ro", uri=True, timeout=1) as db:
            return db.execute(sql, parameters).fetchall()

    def mark(self) -> int:
        self.log.seek(0, 2)
        return self.query("SELECT COALESCE(MAX(trace_id),0) FROM traces")[0][0]

    def drain(self, previous_trace: int) -> dict:
        started = time.monotonic()
        deadline = started + self.settings["drain_timeout_seconds"]
        completed: set[int] = set()
        fragment = ""
        traces = []
        while time.monotonic() < deadline:
            self.cpu.read_ms()  # Fail immediately if the daemon died.
            fragment += self.log.read()
            lines = fragment.split("\n")
            fragment = lines.pop()
            for line in lines:
                match = re.search(r"trace_finalization completed trace_id=trace-(\d+)\b", line)
                if match:
                    completed.add(int(match[1]))
            traces = self.query(
                "SELECT trace_id,lifecycle_state,health FROM traces WHERE trace_id>?", (previous_trace,)
            )
            if traces and all(row[0] in completed for row in traces):
                return {"drain_ms": (time.monotonic() - started) * 1000, "traces": traces}
            time.sleep(self.settings["poll_seconds"])
        raise TimeoutError(f"trace finalization exceeded {self.settings['drain_timeout_seconds']:g}s: {traces}")

    def evidence(self, previous_trace: int, kind: str, directory: Path, *, require_llm_capture=True) -> dict:
        traces = self.query(
            "SELECT trace_id,lifecycle_state,health FROM traces WHERE trace_id>?", (previous_trace,)
        )
        if len(traces) != 1:
            raise RuntimeError(f"expected one trace per cell, got {traces}")
        if str(traces[0][2]).lower() != "clean":
            raise RuntimeError(f"trace is not clean: {traces}")
        events = dict(self.query(
            "SELECT kind_code,COUNT(*) FROM events WHERE trace_id>? GROUP BY kind_code", (previous_trace,)
        ))
        if not events:
            raise RuntimeError("observed workload produced no recorded events")
        actions = dict(self.query(
            "SELECT kind_code,COUNT(*) FROM semantic_actions WHERE trace_id>? GROUP BY kind_code", (previous_trace,)
        ))
        if kind == "agent":
            expected = self.settings["agent_turns"]
            if require_llm_capture and (actions.get(110, 0) != expected or actions.get(111, 0) != expected):
                raise RuntimeError(f"expected {expected} captured LLM requests and responses: {actions}")
        elif kind in ("read", "write"):
            required = 103 if kind == "read" else 104
            count = self.query(
                "SELECT COUNT(*) FROM semantic_actions AS a JOIN file_paths AS p "
                "ON p.trace_id=a.trace_id AND p.path_id=a.file_path_id "
                "WHERE a.trace_id>? AND a.kind_code=? AND p.path_text=?",
                (previous_trace, required, str(directory / "io.bin")),
            )[0][0]
            if not count:
                raise RuntimeError(f"missing captured {kind} action for {directory / 'io.bin'}")
        elif kind in ("idle", "stdio"):
            if not events.get(0, 0):
                raise RuntimeError("idle workload produced no process events")
        elif events.get(0, 0) < self.settings[f"{kind}_operations"]:
            raise RuntimeError(f"too few process events for {kind}: {events}")
        evidence = {"traces": traces, "event_counts": events, "action_counts": actions}
        if kind == "stdio":
            config = tomllib.loads(self.config.read_text())
            stdio = config["payload"]["stdio"]
            if (not stdio["enabled"] or not stdio["capture_stdout"]
                    or "stdio-chunk" not in config["capture"]["capabilities"]):
                raise RuntimeError("stdio benchmark requires stdout capture and stdio-chunk capability")
            evidence["stdio"] = {
                "capture_stdout": True,
                "payload_bytes_verified": False,
                "scope": "workload output and clean trace; dropped stdout has no per-write stored payload",
            }
        if kind in ("read", "write"):
            evidence["workload_file_actions"] = count
        return evidence
