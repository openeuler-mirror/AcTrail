#!/usr/bin/env python3
"""Real v1 container accounting, xiaoo execution, recovery and finalization."""
from __future__ import annotations

import argparse
import importlib.util
import json
import secrets
import select
import sqlite3
import subprocess
import sys
import tempfile
import time
import tomllib
from pathlib import Path

ROOT = Path(__file__).resolve().parents[3]
SPEC = importlib.util.spec_from_file_location("resource_acceptance", Path(__file__).with_name("e2e.py"))
HARNESS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(HARNESS)


class V1Acceptance(HARNESS.ResourceProfilingAcceptance):
    def wait_for(self, predicate, message, timeout=60):
        deadline = time.monotonic() + timeout
        while time.monotonic() < deadline:
            if predicate():
                return
            time.sleep(0.2)
        raise TimeoutError(message)

    def query(self, sql, parameters=()):
        with sqlite3.connect(self.work / "actrail.sqlite") as connection:
            return connection.execute(sql, parameters).fetchall()

    def events(self, trace):
        result = self.run_command([
            self.bins / "actrailviewer", "--config", self.config, "--output-format", "json",
            "workload-resources", "--limit", "10000", "--trace-id", f"trace-{trace}",
        ])
        return json.loads(result)

    def count(self, trace):
        return len(self.events(trace))

    def start_daemon(self):
        self.daemon = subprocess.Popen(
            [str(self.bins / "actraild"), "--config", str(self.config), "run"],
            cwd=self.work, env=self.environment, stdout=self.log, stderr=self.log,
        )
        def ready():
            if self.daemon.poll() is not None:
                raise RuntimeError("daemon startup failed; see daemon.log")
            result = subprocess.run(
                [str(self.bins / "actrailctl"), "--config", str(self.config), "doctor"],
                stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, timeout=10,
            )
            return result.returncode == 0
        self.wait_for(ready, "daemon not ready")

    def restart(self):
        self.daemon.kill()
        self.daemon.wait(timeout=10)
        # SIGKILL leaves this isolated test's Unix datagram endpoint behind.
        (self.work / "tls-sync.sock").unlink(missing_ok=True)
        self.start_daemon()

    def controls(self, boundaries):
        files = ((boundaries[0], ("cpu.shares", "cpu.cfs_period_us", "cpu.cfs_quota_us")),
                 (boundaries[1], ("memory.limit_in_bytes", "memory.soft_limit_in_bytes", "memory.use_hierarchy")))
        return {str(path / name): (path / name).read_text()
                for path, names in files for name in names if (path / name).exists()}

    def start_provider(self, marker):
        self.provider = subprocess.Popen([
            sys.executable, str(ROOT / "tests/support/llm-http-proxy/provider_proxy.py"),
            "--mode", "local-stream", "--bind-host", "127.0.0.1", "--bind-port", "0",
            "--local-stream-response-text", marker, "--local-stream-reasoning-tokens", "3",
            "--response-chunk-delay-seconds", "0.05",
        ], stdout=subprocess.PIPE, stderr=self.log, text=True)
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            readable, _, _ = select.select([self.provider.stdout], [], [], 1)
            if readable:
                line = self.provider.stdout.readline()
                if line.startswith("proxy_base_url="):
                    return line.strip().split("=", 1)[1]
            if self.provider.poll() is not None:
                break
        raise RuntimeError("local provider did not start")

    def run(self):
        self.daemon = self.provider = None
        self.container = ""
        self.log = (self.out / "daemon.log").open("w")
        directory = tempfile.TemporaryDirectory(prefix="arp-v1-")
        self.work = Path(directory.name)
        try:
            self.exercise()
            self.report["status"] = "passed"
        except BaseException as error:
            self.report.update(status="failed", error=str(error))
            raise
        finally:
            if self.daemon is not None and self.daemon.poll() is None:
                self.daemon.terminate()
                try:
                    self.daemon.wait(timeout=60)
                except subprocess.TimeoutExpired:
                    self.daemon.kill()
                    self.daemon.wait(timeout=10)
            if self.container:
                self.run_command(["docker", "rm", "-f", self.container])
            if self.provider is not None and self.provider.poll() is None:
                self.provider.terminate()
                self.provider.wait(timeout=10)
            if (self.work / "actrail.sqlite").exists():
                with sqlite3.connect(self.work / "actrail.sqlite") as source:
                    with sqlite3.connect(self.out / "evidence.sqlite") as target:
                        source.backup(target)
            self.log.close()
            directory.cleanup()
            (self.out / "report.json").write_text(json.dumps(self.report, indent=2) + "\n")

    def exercise(self):
        self.config = self.work / "operator.toml"
        self.init(self.config, "--force")
        capabilities = tomllib.loads(self.config.read_text())["capture"]["capabilities"]
        capabilities = [name for name in capabilities if not name.startswith("enforcement-")]
        patch = self.write_paths(self.work)
        with patch.open("a") as output:
            output.write('\n[capture]\ncapabilities=' + json.dumps(capabilities) + '\n')
            for branch in ("file.open", "command.exec", "network.connect"):
                output.write(f'\n[access_control.{branch}]\nenabled=false\n')
            output.write('\n[resource_profiling.collection.workload]\nmode="cgroup"\n'
                         '[resource_profiling.collection.workload.cgroup]\nmode="v1"\n'
                         '[resource_profiling.collection.workload.cgroup.v2.managed]\nroot="inactive-invalid"\n')
        self.init(self.config, "--force", "--patch", patch)
        (self.out / "operator.toml").write_bytes(self.config.read_bytes())
        marker = "V1_REAL_XIAOO_" + secrets.token_hex(8)
        url = self.start_provider(marker)
        (self.work / "xiaoo.toml").write_text(
            '[llm]\nprovider="deepseek"\nmodel="deepseek-chat"\n'
            'api_key_env="ACTRAIL_V1_TEST_KEY"\n' + f'api_base={json.dumps(url)}\n'
            'max_tokens=128\ncontext_window=32768\nreasoning_effort="off"\n')
        (self.work / "load.py").write_text(
            'import time\nmemory=bytearray(64*1024*1024)\n'
            'while True:\n    sum(range(100000))\n    time.sleep(0.01)\n')
        # The real agent is a descendant of the attached container entrypoint.
        (self.work / "run.sh").write_text(
            '#!/bin/sh\nwhile [ ! -f /work/start ]; do sleep 0.1; done\n'
            'python3 /work/load.py &\n'
            '/opt/xiaoo --cli run --config /work/xiaoo.toml --no-tools --max-turns 1 '
            '--prompt "Report the completion marker" > /work/agent.log 2>&1\n'
            'echo $? > /work/agent.exit\nwait\n')
        image_id = self.run_command(["docker", "image", "inspect", "--format", "{{.Id}}", self.args.image]).strip()
        self.container = self.run_command([
            "docker", "run", "-d", "--network=host",
            "--mount", f"type=bind,src={self.agent.resolve()},dst=/opt/xiaoo,readonly",
            "--mount", f"type=bind,src={self.work},dst=/work",
            "--env", "ACTRAIL_V1_TEST_KEY=local-e2e", "--entrypoint", "/bin/sh",
            image_id, "/work/run.sh",
        ]).strip()
        pid = int(self.run_command(["docker", "inspect", "--format", "{{.State.Pid}}", self.container]))
        membership = Path(f"/proc/{pid}/cgroup").read_text()
        self.start_daemon()
        control = [self.bins / "actrailctl", "--config", self.config]
        self.run_command([*control, "track-add", "--pid", pid, "--name", "v1-agent-recovery"])
        row = self.query("SELECT cpuacct,memory FROM trace_cgroup_v1_bindings WHERE trace_id=1")[0]
        identities = [json.loads(value) for value in row]
        boundaries = [Path(value["mount_point"]) / value["relative_path"] for value in identities]
        if any(self.container not in str(path) for path in boundaries):
            raise AssertionError("binding must refer to the disposable container")
        before = self.controls(boundaries)
        (self.work / "start").touch()
        self.wait_for(lambda: (self.work / "agent.exit").exists(), "real xiaoo did not complete")
        agent_output = (self.work / "agent.log").read_text()
        (self.out / "agent.log").write_text(agent_output)
        if (self.work / "agent.exit").read_text().strip() != "0" or marker not in agent_output:
            raise AssertionError("real xiaoo execution failed")
        self.wait_for(lambda: self.count(1) >= 3, "initial v1 samples missing")
        count = self.count(1)
        self.restart()
        memory_readings = [int((boundaries[1] / "memory.usage_in_bytes").read_text())]
        cpu_start = int((boundaries[0] / "cpuacct.usage").read_text()) // 1000
        self.wait_for(lambda: self.count(1) >= count + 3, "v1 samples did not recover")
        memory_readings.append(int((boundaries[1] / "memory.usage_in_bytes").read_text()))
        cpu_end = int((boundaries[0] / "cpuacct.usage").read_text()) // 1000
        events = sorted(self.events(1), key=lambda e: e["observed_at"]["secs_since_epoch"] * 1_000_000_000 + e["observed_at"]["nanos_since_epoch"])
        self.validate_cgroup_samples(events)
        recovered = events[count:]
        if recovered[0]["payload"]["cpu_percent_millis"] is not None:
            raise AssertionError("restart must reset monotonic CPU baseline")
        low, high = min(memory_readings) - 8*1024**2, max(memory_readings) + 8*1024**2
        for event in recovered:
            payload = event["payload"]
            if not low <= payload["memory_current_bytes"] <= high:
                raise AssertionError("v1 memory disagrees with direct kernel counter")
        if not any(cpu_start <= event["payload"]["cpu_usage_usec"] <= cpu_end for event in recovered):
            raise AssertionError("v1 CPU units disagree with direct kernel counter")
        if self.query("SELECT cpuacct,memory FROM trace_cgroup_v1_bindings WHERE trace_id=1")[0] != row:
            raise AssertionError("recovery changed durable boundary identities")
        children = Path(f"/proc/{pid}/task/{pid}/children").read_text().split()
        load_pids = [int(child) for child in children
                     if b"/work/load.py" in Path(f"/proc/{child}/cmdline").read_bytes().split(b"\0")]
        if len(load_pids) != 1:
            raise AssertionError("expected one live container load process")
        # Removing this leaf has no live descendants keeping the trace draining.
        self.run_command([*control, "track-add", "--pid", load_pids[0], "--name", "v1-agent-removal"])
        self.wait_for(lambda: self.count(2) >= 2, "second trace samples missing")
        self.run_command([*control, "track-remove", "--trace-id", "trace-2"])
        self.wait_for(lambda: self.query("SELECT closed_at IS NOT NULL FROM trace_cgroup_v1_bindings WHERE trace_id=2") == [(1,)], "live explicit removal did not close")
        if self.controls(boundaries) != before or Path(f"/proc/{pid}/cgroup").read_text() != membership:
            raise AssertionError("read-only profiling changed controls or membership")
        self.run_command(["docker", "stop", "--time", "1", self.container])
        self.wait_for(lambda: self.query(
            "SELECT b.closed_at IS NOT NULL,t.lifecycle_state FROM trace_cgroup_v1_bindings b "
            "JOIN traces t USING(trace_id) WHERE trace_id=1") == [(1, "completed")],
            "recovered trace did not finalize after workload exit")
        for trace in (1, 2):
            finals = [e for e in self.events(trace) if e["payload"]["sample_kind"] == "final"]
            if len(finals) != 1:
                raise AssertionError("expected exactly one durable final per trace")
            if trace == 2 and not finals[0]["payload"]["memory_current_bytes"]:
                raise AssertionError("live removal lost readable final counters")
        self.restart()
        time.sleep(1.2)
        for trace in (1, 2):
            events = self.events(trace)
            (self.out / f"trace-{trace}-events.json").write_text(json.dumps(events, indent=2))
            if len([e for e in events if e["payload"]["sample_kind"] == "final"]) != 1:
                raise AssertionError("restart duplicated final event")
        self.report.update(real_agent_completed=True, provider="local-stream", accounting_method="cgroup_v1",
                           container=self.container, cpu_counter_bounds_usec=[cpu_start, cpu_end],
                           memory_counter_readings=memory_readings, controls_unchanged=True,
                           recovery=True, final_once=True, inactive_v2_config_ignored=True)
        print("v1: real container xiaoo, kernel counters, recovery and finalization passed", flush=True)


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", type=Path, default=ROOT / "target/release")
    parser.add_argument("--xiaoo", type=Path)
    parser.add_argument("--image", required=True)
    parser.add_argument("--out", type=Path, required=True)
    V1Acceptance(parser.parse_args()).run()
