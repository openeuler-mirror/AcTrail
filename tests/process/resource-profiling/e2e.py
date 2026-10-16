#!/usr/bin/env python3
"""Exercise resource branch isolation through refreshed defaults and a real xiaoO."""

from __future__ import annotations

import argparse
import importlib.util
import json
import re
import secrets
import subprocess
import tempfile
import time
import tomllib
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location(
    "config_init_acceptance", ROOT / "tests/process/config-init/e2e.py",
)
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class ResourceProfilingAcceptance(HARNESS.ConfigInitAcceptance):
    def run(self):
        try:
            cases = [("auto-tree", True), ("procfs-tree", True), ("procfs-root", False), ("host-only", True), ("workload-only", True), ("total-off", True), ("host-noop", True), ("workload-noop", True), ("both-noop", True)]
            if not any(line.startswith("0::") for line in Path("/proc/self/cgroup").read_text().splitlines()):
                cases.extend([("v1-explicit", True), ("auto-v2-unavailable", True)])
            for name, include_children in cases:
                with tempfile.TemporaryDirectory(prefix="arp-") as directory:
                    self.run_case(name, include_children, Path(directory))
            self.validate_startup_failures()
            self.report["status"] = "passed"
        except BaseException as error:
            self.report["status"] = "failed"
            self.report["error"] = str(error)
            raise
        finally:
            (self.out / "report.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def run_case(self, name: str, include_children: bool, work: Path):
        expect_host_samples = name not in ("workload-only", "total-off", "host-noop", "both-noop")
        expect_workload_samples = name not in ("host-only", "total-off", "workload-noop", "both-noop")
        config = work / "operator.toml"
        patch = self.write_paths(work)
        if name in ("host-only", "total-off"):
            self.init(config, "--force")
            capabilities = tomllib.loads(config.read_text())["capture"]["capabilities"]
            capabilities.remove("resource-metrics")
            with patch.open("a") as output:
                output.write('\n[capture]\ncapabilities = ' + json.dumps(capabilities) + '\n')
        if not include_children:
            with patch.open("a") as output:
                output.write('\n[resource_profiling.collection.workload]\nmode = "procfs"\n[resource_profiling.collection.workload.procfs]\ninclude_children = false\n[resource_profiling.collection.workload.cgroup.v2.managed]\nroot = "inactive-invalid-path"\n')
        with patch.open("a") as output:
            if name == "procfs-tree":
                output.write('\n[resource_profiling.collection.workload]\nmode = "procfs"\n')
            if name == "v1-explicit":
                output.write('\n[resource_profiling.collection.workload]\nmode = "cgroup"\n[resource_profiling.collection.workload.cgroup]\nmode = "v1"\n[resource_profiling.collection.workload.cgroup.v2.managed]\nroot = "inactive-invalid"\n')
            if name == "auto-v2-unavailable":
                output.write('\n[resource_profiling.collection.workload.cgroup]\nmode = "v2"\n')
            if name in ("host-noop", "both-noop"):
                output.write('\n[resource_profiling.collection.host]\nmode = "noop"\n')
            if name in ("workload-noop", "both-noop"):
                output.write('\n[resource_profiling.collection.workload]\nmode = "noop"\n[resource_profiling.collection.workload.procfs]\ninclude_children = "inactive-invalid"\n[resource_profiling.collection.workload.cgroup.v2.managed]\nroot = "inactive-invalid-path"\n')
            if name == "host-only":
                output.write('\n[resource_profiling.collection.workload]\nenabled = false\nmode = "inactive-invalid-mode"\n')
                output.write('\n[storage.retention]\nenabled = false\n[resource_profiling.storage.database.sqlite.retention]\nmax_age_seconds = 3\nsweep_interval_seconds = 1\nmax_samples_per_sweep = 2\n')
            if name == "workload-only":
                output.write('\n[resource_profiling.collection.host]\nenabled = false\n')
            if name == "procfs-root":
                output.write('\n[resource_profiling.collection.host]\ninterval_ms = 250\n')
            if name == "total-off":
                output.write('\n[resource_profiling]\nenabled = false\n[resource_profiling.collection.host]\ninterval_ms = 0\n[resource_profiling.collection.workload]\nmode = "inactive-invalid-mode"\n')
        self.init(config, "--force", "--patch", patch)
        (self.out / f"{name}.toml").write_bytes(config.read_bytes())
        marker = "RESOURCE_PROFILING_" + secrets.token_hex(8)
        self.write_workload(work, marker)
        prompt = (
            "Use the bash tool exactly once to execute `python3 workload.py`. "
            "Wait for it to finish. Reply with only the completion marker printed by the program."
        )
        control = [self.bins / "actrailctl", "--config", config]
        trace = None
        with (self.out / f"{name}-daemon.log").open("w") as log:
            daemon = subprocess.Popen(
                [str(self.bins / "actraild"), "--config", str(config), "run"],
                cwd=work, env=self.environment, stdout=log, stderr=subprocess.STDOUT,
                start_new_session=True,
            )
            try:
                deadline = time.monotonic() + 60
                while not (work / "control.sock").exists():
                    if daemon.poll() is not None:
                        raise RuntimeError(f"{name}: daemon exited during startup")
                    if time.monotonic() >= deadline:
                        raise TimeoutError(f"{name}: daemon startup exceeded 60 seconds")
                    time.sleep(0.1)
                time.sleep(2.2)
                before = [] if name == "total-off" else self.host_samples(config)
                if (len(before) >= 2) != expect_host_samples:
                    raise AssertionError("independent host sampling before any trace failed")
                launch = self.run_command(
                    [*control, "launch", "--", self.agent, "--cli", "run", "--tools", "bash",
                     "--max-turns", "4", "--prompt", prompt],
                    cwd=work, timeout=self.args.launch_timeout,
                )
                result = json.loads((work / "workload-result.json").read_text())
                if result["marker"] != marker or marker not in launch:
                    raise AssertionError(f"{name}: real agent did not finish the workload")
                traces = re.findall(r"trace trace-(\d+) entered Active", launch)
                if len(traces) != 1:
                    raise AssertionError(f"{name}: expected one trace, got {traces}")
                trace = traces[0]
                (self.out / f"{name}-workload.json").write_text(json.dumps(result, indent=2))
            finally:
                if daemon.poll() is None:
                    daemon.terminate()
                    try:
                        daemon.wait(timeout=240)
                    except subprocess.TimeoutExpired:
                        self.terminate(daemon)
                if daemon.returncode != 0:
                    raise RuntimeError(f"{name}: daemon exited with {daemon.returncode}")
        events_text = "[]" if name == "total-off" else self.run_command(
            [self.bins / "actrailviewer", "--config", config, "--output-format", "json",
             "workload-resources", "--limit", "10000", "--trace-id", f"trace-{trace}"], timeout=60,
        )
        (self.out / f"{name}-events.json").write_text(events_text)
        events = json.loads(events_text)
        events.sort(key=lambda event: event["observed_at"]["secs_since_epoch"] * 1_000_000_000 + event["observed_at"]["nanos_since_epoch"])
        if not expect_workload_samples:
            if events:
                raise AssertionError("workload backend expected no samples")
            summary = {"samples": 0}
        else:
            if events and events[0]["payload"]["accounting_method"].startswith("cgroup_"):
                summary = self.validate_cgroup_samples(events)
                if summary["accounting_method"] == "cgroup_v1":
                    finals = [e["payload"] for e in events if e["payload"]["sample_kind"] == "final"]
                    if len(finals) != 1 or not finals[0]["memory_current_bytes"]:
                        raise AssertionError("normal process exit lost the pinned v1 final counters")
            else:
                summary = self.validate_samples(events, result, include_children)
            if name == "v1-explicit" and summary.get("accounting_method") != "cgroup_v1":
                raise AssertionError("explicit v1 did not select the v1 backend")
            if name == "auto-v2-unavailable" and events[0]["payload"]["accounting_method"] != "procfs_rss_sum":
                raise AssertionError("fixed v2 candidate incorrectly crossed into v1")
        hosts = [] if name == "total-off" else self.host_samples(config)
        (self.out / f"{name}-host.json").write_text(json.dumps(hosts, indent=2))
        if not expect_host_samples:
            if hosts:
                raise AssertionError("host backend expected no samples")
        else:
            if len(hosts) < 3 or not any(sample["cpu_percent_millis"] is not None for sample in hosts):
                raise AssertionError("host CPU baseline did not become ready")
            boot_id = Path("/proc/sys/kernel/random/boot_id").read_text().strip()
            for sample in hosts:
                if sample["host_boot_id"] != boot_id or sample["memory_total_bytes"] <= 0:
                    raise AssertionError("invalid host identity or memory")
                if sample["cpu_percent_millis"] is not None and not 0 <= sample["cpu_percent_millis"] <= 100000:
                    raise AssertionError("host CPU must be normalized to machine capacity")
            if name == "procfs-root" and len(hosts) < len(events) * 2:
                raise AssertionError("host sampling interval is coupled to workload interval")
            if name == "host-only":
                times = [int(sample["observed_at_unix_nanos"]) for sample in hosts]
                if max(times) - min(times) > 4_000_000_000:
                    raise AssertionError("host retention did not expire real samples independently")
        summary["real_agent_completed"] = True
        summary["host_samples"] = len(hosts)
        summary["host_samples_before_trace"] = len(before)
        self.report["modes"][name] = summary
        print(f"{name}: real xiaoO resource samples passed ({len(events)} samples)", flush=True)

    @staticmethod
    def validate_cgroup_samples(events):
        periodic = [event["payload"] for event in events if event["payload"]["sample_kind"] == "periodic"]
        if len(periodic) < 3 or periodic[0]["cpu_percent_millis"] is not None:
            raise AssertionError("cgroup periodic sampling or initial baseline failed")
        if not any((sample["cpu_percent_millis"] or 0) > 0 for sample in periodic):
            raise AssertionError("cgroup CPU baseline did not measure activity")
        for sample in periodic:
            if sample["memory_current_bytes"] <= 0 or sample["cpu_usage_usec"] < 0:
                raise AssertionError("invalid cgroup counters")
            if sample["accounting_method"] == "cgroup_v1":
                if sample["accounting_coverage"] != "broader_than_trace" or sample["rss_kb"] is not None:
                    raise AssertionError("v1 shared charged memory must retain its scope and units")
        return {"samples": len(events), "periodic_samples": len(periodic),
                "accounting_method": periodic[0]["accounting_method"]}

    def validate_startup_failures(self):
        cases = {
            "invalid-cgroup-mode": ('[resource_profiling.collection.workload.cgroup]\nmode = "hybrid"\n', "mode"),
            "invalid-v1-attempts": ('[resource_profiling.collection.workload.cgroup.v1]\nbinding_attempts = 0\n', "binding_attempts"),
            "invalid-host-mode": ('[resource_profiling.collection.host]\nmode = "invalid"\n', "mode"),
            "invalid-workload-mode": ('[resource_profiling.collection.workload]\nmode = "invalid"\n', "mode"),
            "invalid-managed-root": ('[resource_profiling.collection.workload.cgroup.v2.managed]\nroot = "relative"\n', "root"),
            "invalid-host-interval": ('[resource_profiling.collection.host]\ninterval_ms = 0\n', "interval_ms"),
            "invalid-external-attempts": ('[resource_profiling.collection.workload.cgroup.v2.external]\npolicy = "prefer"\nbinding_attempts = 0\n', "binding_attempts"),
        }
        for name, (content, expected) in cases.items():
            with tempfile.TemporaryDirectory(prefix="arp-invalid-") as directory:
                work = Path(directory)
                patch = self.write_paths(work)
                with patch.open("a") as output:
                    output.write("\n" + content)
                result = self.init(work / "operator.toml", "--force", "--patch", patch, success=False)
                if expected not in result:
                    raise AssertionError(f"{name}: rejection did not identify its owning configuration")
        if not any(line.startswith("0::") for line in Path("/proc/self/cgroup").read_text().splitlines()):
            with tempfile.TemporaryDirectory(prefix="arp-cgroup-v1-") as directory:
                work = Path(directory)
                patch = self.write_paths(work)
                with patch.open("a") as output:
                    output.write('\n[resource_profiling.collection.workload]\nmode = "cgroup"\n[resource_profiling.collection.workload.cgroup]\nmode = "v2"\n')
                config = work / "operator.toml"
                self.init(config, "--force", "--patch", patch)
                result = self.run_command([self.bins / "actraild", "--config", config, "run"], cwd=work, success=False)
                if "cgroup" not in result:
                    raise AssertionError("explicit cgroup-v2 rejection lacks its cause")
            self.report["cgroup_v2"] = "unavailable on this cgroup-v1 host; explicit mode startup rejection passed"
        self.report["startup_validation"] = list(cases)

    def host_samples(self, config):
        return json.loads(self.run_command([
            self.bins / "actrailviewer", "--config", config, "--output-format", "json",
            "host-resources", "--limit", "1000",
        ], timeout=60))

    @staticmethod
    def write_workload(work: Path, marker: str):
        source = '''import json
import os
import time
from pathlib import Path

time.sleep(2)
memory = bytearray(128 * 1024 * 1024)
for offset in range(0, len(memory), 4096):
    memory[offset] = 1
started = time.time_ns()
deadline = time.monotonic() + 8
while time.monotonic() < deadline:
    sum(range(100000))
ended = time.time_ns()
rss = next(line for line in Path("/proc/self/status").read_text().splitlines()
           if line.startswith("VmRSS:"))
result = {"pid": os.getpid(), "started_ns": started, "ended_ns": ended,
          "rss_kb": int(rss.split()[1]), "marker": MARKER}
Path("workload-result.json").write_text(json.dumps(result))
print(MARKER, flush=True)
'''
        (work / "workload.py").write_text("MARKER = " + repr(marker) + "\n" + source)

    @staticmethod
    def validate_samples(events: list[dict], workload: dict, include_children: bool):
        if len(events) < 3:
            raise AssertionError("expected at least three persisted resource samples")
        first = events[0]["payload"]
        if first["cpu_percent_millis"] is not None or first["metadata"]["cpu_baseline_ready"] != "false":
            raise AssertionError("first sample must report an uninitialized CPU baseline")
        ready = [event for event in events if event["payload"]["cpu_percent_millis"] is not None]
        if not ready or any(event["payload"]["metadata"]["cpu_baseline_ready"] != "true" for event in ready):
            raise AssertionError("subsequent CPU samples did not establish a baseline")
        expected_scope = "process_tree" if include_children else "process"
        for event in events:
            sample = event["payload"]
            if sample["accounting_method"] != "procfs_rss_sum" or sample["scope"] != expected_scope:
                raise AssertionError("wrong workload accounting method or scope")
            if sample["rss_kb"] <= 0 or sample["rss_kb"] != sample["process_rss_sum_kb"]:
                raise AssertionError("invalid RSS sample")
            if sample["metadata"]["include_children"] != str(include_children).lower():
                raise AssertionError("wrong procfs branch configuration")
            if not include_children and sample["metadata"]["sampled_processes"] != "1":
                raise AssertionError("root-only mode included another process")
        during = [event["payload"] for event in events
                  if workload["started_ns"] <= (event["observed_at"]["secs_since_epoch"] * 1_000_000_000 + event["observed_at"]["nanos_since_epoch"]) <= workload["ended_ns"]]
        if len(during) < 3:
            raise AssertionError("insufficient samples during the actual memory/CPU workload")
        if include_children:
            if not any(int(sample["metadata"]["sampled_processes"]) >= 2
                       and sample["rss_kb"] >= workload["rss_kb"] for sample in during):
                raise AssertionError("process-tree samples do not include the real child workload")
            if not any((sample["cpu_percent_millis"] or 0) > 0 for sample in during):
                raise AssertionError("CPU load was not measured")
        return {"samples": len(events), "baseline_samples": len(ready),
                "workload_samples": len(during), "scope": expected_scope,
                "max_rss_kb": max(sample["rss_kb"] for sample in during),
                "max_cpu_percent_millis": max(sample["cpu_percent_millis"] or 0 for sample in during)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--xiaoo", type=Path)
    parser.add_argument("--out", type=Path, required=True)
    parser.add_argument("--launch-timeout", type=int, default=240)
    ResourceProfilingAcceptance(parser.parse_args()).run()
