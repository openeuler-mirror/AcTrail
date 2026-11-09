"""Verify retained kernel resources using repeated real Xiaoo launches."""

from __future__ import annotations

import json
import os
import re
import subprocess
import time
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestResult, TestStatus


class KernelResources:
    def __init__(self, pid: int):
        self.ids = {kind: set() for kind in ("prog", "map", "link")}
        for entry in Path(f"/proc/{pid}/fdinfo").iterdir():
            try:
                content = entry.read_text()
            except FileNotFoundError:
                continue
            for kind, identifier in re.findall(r"^(prog|map|link)_id:\s+(\d+)$", content, re.M):
                self.ids[kind].add(int(identifier))

    def require_same(self, other: KernelResources) -> None:
        if self.ids != other.ids:
            raise AssertionError(f"kernel resources changed: {self.json()} -> {other.json()}")

    def json(self) -> dict:
        return {kind: sorted(values) for kind, values in self.ids.items()}

    def require_released(self, timeout: float, interval: float) -> None:
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            retained = False
            for kind, identifiers in self.ids.items():
                if not identifiers:
                    continue
                result = subprocess.run(["bpftool", "-j", kind, "show"], capture_output=True, text=True, check=True)
                live = {item["id"] for item in json.loads(result.stdout)}
                retained |= bool(identifiers.intersection(live))
            if not retained:
                return
            time.sleep(interval)
        raise AssertionError("daemon BPF resources remained after shutdown")


class MeasuredRuntime(ActrailRuntime):
    start_ms: float = 0

    def run(self, argv, **kwargs):
        started = time.monotonic_ns()
        result = super().run(argv, **kwargs)
        if str(argv[0]) == str(self.actraild) and str(argv[-1]) == "start":
            self.start_ms = (time.monotonic_ns() - started) / 1_000_000
        return result


class StartupReuseScenario:
    def __init__(self, case):
        self.case = case
        self.baseline = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_STARTUP_BASELINE") == "1"
        self.samples = int(os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_STARTUP_SAMPLES", "3"))
        if self.samples <= 0:
            raise ValueError("startup sample count must be positive")

    def run(self, context, results: dict, port: int) -> None:
        case = self.case
        case._write_patch()
        reports = []
        for sample in range(self.samples):
            context.report_progress("startup_reuse", f"{case._selection.name}: sample {sample + 1}")
            # Each sample refreshes defaults and creates an independent daemon.
            case._write_rules("allow", port)
            runtime = MeasuredRuntime(
                case._config.repo, case._config.bin_dir, case._config.command_timeout_seconds,
                context.output, case._work / "actraild.conf", operator_config_patches=case._patches(),
            )
            case._runtime = runtime
            runtime.prepare()
            runtime.run_checked(runtime.control_command("list-traces"), echo=False)
            pid = int((case._work / "run/actraild.pid").read_text().strip())
            initial = KernelResources(pid)
            if not initial.ids["prog"] or not initial.ids["map"]:
                raise AssertionError("prepared daemon has no BPF program/map FDs")
            launch_ms = []
            for _ in range(2):
                started = time.monotonic_ns()
                trace = case._run_xiaoo("allow")
                launch_ms.append((time.monotonic_ns() - started) / 1_000_000)
                case._require_workload("allow")
                case._require_evidence(trace, "allow")
                initial.require_same(KernelResources(pid))
            log = (case._work / "log/actraild.log").read_text()
            attempts = re.findall(r"host_ebpf_preparation started", log)
            loads = re.findall(r"ebpf_runtime_load started", log)
            prepared = re.findall(r"host_ebpf_preparation completed.*elapsed_ms=(\d+)", log)
            if not self.baseline and (len(attempts) != 1 or len(prepared) != 1 or len(loads) != 1):
                raise AssertionError(f"expected one load, attempts={len(attempts)} results={len(prepared)} loads={len(loads)}")
            reports.append({
                "start_ms": runtime.start_ms,
                "launch_ms": launch_ms,
                "preparation_ms": int(prepared[0]) if prepared else None,
                "preparation_attempts": len(attempts) if not self.baseline else None,
                "runtime_loads": len(loads) if not self.baseline else None,
                "resources": initial.json(),
            })
            stopped = runtime.stop()
            if stopped is None or stopped.returncode != 0:
                raise AssertionError("daemon did not stop cleanly")
            initial.require_released(case._timeout, case._poll)
            case._runtime = None
        report = {"backend": case._selection.name, "baseline": self.baseline, "samples": reports}
        context.output.line("STARTUP_REUSE_EVIDENCE=" + json.dumps(report, sort_keys=True))
        results[case._selection.name] = TestResult(TestStatus.PASSED, "real Xiaoo launches retained program, link and map IDs")
