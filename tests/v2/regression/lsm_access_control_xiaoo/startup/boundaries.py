"""Exercise empty preparation, real privilege failure and restart recovery."""

from __future__ import annotations

import json
import os
import re
import signal
import subprocess
import time
from pathlib import Path

from tests.v2.common.core import TestResult, TestStatus
from .reuse import KernelResources, MeasuredRuntime
from ..constants import CASE_CONSTANTS


class RestrictedRuntime(MeasuredRuntime):
    def run(self, argv, **kwargs):
        if str(argv[0]) == str(self.actraild) and str(argv[-1]) == "start":
            argv = ["setpriv", "--bounding-set=-bpf,-perfmon,-sys_admin", *argv]
        return super().run(argv, **kwargs)


class StartupBoundariesScenario:
    def __init__(self, case):
        self.case = case

    def run(self, context, results: dict, port: int) -> None:
        case = self.case
        case._write_patch()
        case._write_rules("allow", port)
        patch = case._work / "startup-boundaries.toml"
        self._write_patch(patch, observation=False)
        runtime = self._runtime(MeasuredRuntime, context, patch)
        runtime.prepare()
        self._require_empty()
        self._launch(host="disabled")
        self._require_empty()
        log = self._log()
        if "host_ebpf_preparation started" in log or "ebpf_runtime_load started" in log:
            raise AssertionError("empty configuration attempted eBPF preparation")
        self._stop(runtime)
        results["empty"] = TestResult(TestStatus.PASSED, "real Xiaoo ran without daemon BPF FDs")

        self._write_patch(patch, observation=True)
        runtime = self._runtime(RestrictedRuntime, context, patch)
        runtime.prepare()
        self._require_empty()
        before = self._log()
        for _ in range(2):
            result = runtime.run(runtime.control_command(
                "launch", "--host-ebpf", "required", "--seccomp-notify", "disabled",
                "--", case._xiaoo, "--version",
            ), echo=False)
            if result.returncode == 0:
                raise AssertionError("required eBPF launch accepted after preparation failed")
            if "host eBPF required but daemon collector is unavailable" not in result.output:
                raise AssertionError(f"unexpected launch rejection: {result.output}")
            runtime.run_checked(runtime.control_command("list-traces"), echo=False)
            self._require_empty()
        self._launch(host="disabled")
        after = self._log()
        if before.count("host_ebpf_preparation started") != 1:
            raise AssertionError("missing initial failed preparation")
        for marker in ("host_ebpf_preparation started", "ebpf_runtime_load started"):
            if before.count(marker) != after.count(marker):
                raise AssertionError(f"repeated request retried {marker}")
        self._stop(runtime)
        results["failed_requests"] = TestResult(TestStatus.PASSED, "real permission failure persisted, repeated requests rejected, fallback Xiaoo succeeded")

        runtime = self._runtime(MeasuredRuntime, context, patch)
        runtime.prepare()
        initial = self._snapshot()
        if not initial.ids["prog"]:
            raise AssertionError("restart with restored privileges did not prepare eBPF")
        self._launch(host="required")
        initial.require_same(self._snapshot())
        self._attach()
        initial.require_same(self._snapshot())
        self._stop(runtime)
        initial.require_released(case._timeout, case._poll)
        results["restart"] = TestResult(TestStatus.PASSED, "restored privileges took effect after restart with real Xiaoo launch and attach")

        from .tls import TlsStartupScenario
        TlsStartupScenario(self).run(context, results, patch)

        # The original LSM configuration is mandatory and must fail startup.
        runtime = self._runtime(RestrictedRuntime, context, None)
        try:
            runtime.prepare()
        except RuntimeError as error:
            if "lsm_preflight" not in str(error) and "lsm_preflight" not in self._log():
                raise
        else:
            self._stop(runtime)
            raise AssertionError("mandatory LSM failure allowed daemon readiness")
        results["mandatory_lsm"] = TestResult(TestStatus.PASSED, "LSM privilege failure rejected startup")
        case._runtime = None

    @staticmethod
    def _stop(runtime):
        stopped = runtime.stop()
        if stopped is None or stopped.returncode:
            raise AssertionError("daemon did not stop cleanly")

    def _runtime(self, kind, context, patch):
        case = self.case
        patches = case._patches()
        if patch is not None:
            patches = [*patches, patch]
        runtime = kind(case._config.repo, case._config.bin_dir, case._config.command_timeout_seconds,
                       context.output, case._work / "actraild.conf", operator_config_patches=patches)
        case._runtime = runtime
        return runtime

    def _snapshot(self):
        return KernelResources(int((self.case._work / "run/actraild.pid").read_text().strip()))

    def _require_empty(self):
        current = self._snapshot()
        if any(current.ids.values()):
            raise AssertionError(f"unexpected kernel resources: {current.json()}")

    def _log(self):
        return (self.case._work / "log/actraild.log").read_text()

    def _launch(self, *, host):
        case = self.case
        self._reset_workload()
        result = case._runtime.run_checked(case._runtime.control_command(
            "launch", "--host-ebpf", host, "--seccomp-notify", "disabled", "--", *self._agent_command(),
        ), environment=self._environment())
        if not re.search(r"trace trace-\d+ entered Active", result.output):
            raise AssertionError("real Xiaoo launch omitted active trace")
        self._require_workload()

    def _reset_workload(self):
        case = self.case
        case._marker.write_text("")
        if case._mkdir.exists():
            case._mkdir.rmdir()
        case._rmdir.mkdir(exist_ok=True)
        case._results.unlink(missing_ok=True)
        case._peer.requests.clear()

    def _environment(self):
        environment = os.environ.copy()
        for name in ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"):
            environment.pop(name, None)
        environment.update(ACTRAIL_XIAOO_LSM_KEY="local-test-key", NO_PROXY="127.0.0.1,localhost", PATH="/usr/bin")
        return environment

    def _agent_command(self):
        return [
            self.case._xiaoo, "--cli", "run", "--config", self.case._xiaoo_config,
            "--tools", "bash", "--max-turns", str(CASE_CONSTANTS.agent_max_turns), "--debug",
            "--prompt", "Use the Bash tool exactly once, then report its operating-system results.",
        ]

    def _require_workload(self):
        workload = json.loads(self.case._results.read_text())
        for name in ("file", "command", "network", "mkdir", "rmdir"):
            if not workload[name]["allowed"]:
                raise AssertionError(f"unexpected workload failure: {workload}")

    def _attach(self):
        case = self.case
        self._reset_workload()
        provider = case._provider._process
        # Hold the owned provider while attaching to the actual running Agent.
        provider.send_signal(signal.SIGSTOP)
        agent = None
        try:
            agent = subprocess.Popen(self._agent_command(), env=self._environment(), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
            attached = case._runtime.run_checked(case._runtime.control_command(
                "track-add", "--pid", str(agent.pid), "--name", "startup-attach", "--tag", "startup-query",
            ))
            match = re.search(r"trace trace-(\d+) entered Active", attached.output)
            if match is None:
                raise AssertionError("existing Xiaoo attach did not become active")
            trace = match.group(1)
            listing = case._runtime.run_checked(case._runtime.control_command("list-traces"))
            if not any(line.split()[0] == f"trace-{trace}" and f"pid={agent.pid}" in line.split()
                       for line in listing.stdout.splitlines() if line.split()):
                raise AssertionError(f"attached trace lost Agent PID association: {listing.output}")
            self._require_selectors(trace, agent.pid)
            provider.send_signal(signal.SIGCONT)
            stdout, stderr = agent.communicate(timeout=case._config.launch_timeout_seconds)
            if agent.returncode:
                raise AssertionError(f"attached Xiaoo failed: {stdout}\n{stderr}")
            self._require_workload()
            self._require_selectors(trace, agent.pid)
            deadline = time.monotonic() + case._timeout
            while time.monotonic() < deadline:
                events = case._runtime.run_checked(case._runtime.viewer_command("events", "--trace-id", trace, "--output-format", "json"), echo=False)
                if any(event.get("collector") == "ebpf" and event.get("variant") == "process"
                       and event.get("trace_id_raw") == int(trace)
                       and event.get("payload", {}).get("operation") in ("fork", "exec", "exit")
                       for event in json.loads(events.stdout)["events"]):
                    return
                time.sleep(case._poll)
            raise AssertionError("attached Agent produced no observations")
        finally:
            provider.send_signal(signal.SIGCONT)
            if agent is not None and agent.poll() is None:
                agent.kill()
                agent.communicate(timeout=case._timeout)

    def _require_selectors(self, trace: str, pid: int):
        runtime = self.case._runtime
        for selector in (("--root-pid", str(pid)), ("--trace-id", trace),
                         ("--name", "startup-attach"), ("--tag-selector", "startup-query")):
            listing = runtime.run_checked(runtime.control_command("list-traces", *selector), echo=False)
            rows = [line.split() for line in listing.stdout.splitlines() if line.split()]
            if len(rows) != 1 or rows[0][0] != f"trace-{trace}" or f"pid={pid}" not in rows[0]:
                raise AssertionError(f"trace selector {selector} returned unexpected rows: {listing.output}")
        # The test driver is running but has never been attached to this daemon.
        missing = runtime.run_checked(runtime.control_command("list-traces", "--root-pid", str(os.getpid())), echo=False)
        if missing.stdout.strip():
            raise AssertionError(f"untracked root PID matched a trace: {missing.output}")

    @staticmethod
    def _write_patch(path: Path, *, observation: bool):
        capabilities = '["proc-lifecycle", "resource-metrics"]' if observation else '["resource-metrics"]'
        text = f'[capture]\nprofile_name = "startup-boundaries"\ncapabilities = {capabilities}\nopportunistic_capabilities = []\n'
        text += '\n[ebpf]\nenabled = true\n'
        for section in ("payload.tls", "payload.socket", "payload.stdio", "payload.mcp", "enforcement",
                        "command_control", "network_control", "process_seccomp", "seccomp_notify", "plugins.startup"):
            text += f'\n[{section}]\nenabled = false\n'
        text += '\n[resource_profiling]\nenabled = true\n[resource_profiling.collection.workload]\nenabled = true\nmode = "procfs"\n'
        path.write_text(text)
