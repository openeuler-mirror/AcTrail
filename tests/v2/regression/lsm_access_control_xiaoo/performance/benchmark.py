from __future__ import annotations

import argparse
import json
import os
import platform
import shlex
import shutil
import sys
import tempfile
import time
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.core import TestOutput
from tests.v2.common.runner import TestingContextSingleton
from ..services import WorkloadProvider
from .paths import ENTRY, OUTPUT_DIR, REPO
from .services import Peer, WasmPublisher

from .constants import CASE_CONSTANTS


class Benchmark:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        xiaoo = args.xiaoo or shutil.which("xiaoo")
        if not xiaoo:
            raise RuntimeError("real Xiaoo binary is unavailable")
        self._xiaoo = Path(xiaoo).resolve()
        self._output = TestOutput(color_mode="never")
        self._peers = [Peer(args.peer_poll_seconds, args.backlog) for _ in range(2)]

    def _patches(self, work: Path, backend: str) -> list[Path]:
        patch = work / "actraild.patch.toml"
        (
            PatchBuilder().with_work_dir(work).with_runtime_timing()
            .with_values({"storage": {"retention": {"enabled": False}}})
            .with_values({"network_control": {"rules_path": str(work / "network.rules")}})
            .build().save(patch)
        )
        (work / "network.rules").write_text("" if self.args.policy_source == "wasm" else (
            f'bench-allow allow connect 127.0.0.1:{self._peers[0].port}\n'
            f'bench-deny deny connect 127.0.0.1:{self._peers[1].port}\n'
        ))
        configuration = Path(__file__).parent / "configuration"
        return [patch, configuration / "patch_config.toml", configuration / backend / "patch_config.toml"]

    def _backend(self, root: Path, backend: str) -> dict:
        work = root / backend
        work.mkdir()
        result_path = work / "measurements.json"
        command = shlex.join([
            str(Path(sys.executable).resolve()), str(ENTRY), "worker",
            "--backend", backend, "--allow-port", str(self._peers[0].port),
            "--deny-port", str(self._peers[1].port), "--count", str(self.args.count),
            "--rounds", str(self.args.rounds), "--warmup", str(self.args.warmup),
            "--timeout", str(self.args.timeout), "--results", str(result_path),
            "--xiaoo", str(self._xiaoo), "--daemon-pid", str(work / "run/actraild.pid"),
        ])
        provider = WorkloadProvider(REPO, work, command, self.args.timeout)
        patches = self._patches(work, backend)
        runtime = ActrailRuntime(
            REPO, self.args.bin_dir, self.args.command_timeout, self._output,
            work / "actraild.conf", operator_config_patches=patches,
        )
        publisher = WasmPublisher(self.args, work, self._output, [peer.port for peer in self._peers], patches) \
            if self.args.policy_source == "wasm" else None
        if publisher is not None:
            runtime = publisher.runtime
        before = self._peers[0].accepted
        try:
            url = provider.start()
            config = work / "xiaoo.toml"
            config.write_text(
                f'[llm]\nprovider = "{CASE_CONSTANTS.llm_provider}"\nmodel = "{CASE_CONSTANTS.llm_model}"\n'
                'api_key_env = "ACTRAIL_NETWORK_BENCHMARK_KEY"\n'
                f'api_base = {json.dumps(url)}\n'
                f'max_tokens = {CASE_CONSTANTS.llm_max_tokens}\ncontext_window = {CASE_CONSTANTS.llm_context_window}\nreasoning_effort = "{CASE_CONSTANTS.llm_reasoning_effort}"\n'
            )
            publication = publisher.prepare_and_publish() if publisher is not None else None
            if publisher is None:
                runtime.prepare()
            environment = os.environ.copy()
            for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
                environment.pop(key, None)
            environment.update(
                NO_PROXY=CASE_CONSTANTS.no_proxy_hosts, no_proxy=CASE_CONSTANTS.no_proxy_hosts,
                ACTRAIL_NETWORK_BENCHMARK_KEY="local-test-key", PATH="/usr/bin",
            )
            result = runtime.run(
                runtime.control_command(
                    "launch", "--seccomp-notify", "required" if backend == "seccomp" else "disabled",
                    "--name", f"network-benchmark-{backend}", "--", self._xiaoo,
                    "--cli", "run", "--config", config, "--tools", "bash", "--max-turns", str(CASE_CONSTANTS.agent_max_turns),
                    "--debug", "--prompt", "Use the Bash tool exactly once and report the result.",
                ), timeout_seconds=self.args.launch_timeout, environment=environment,
            )
            log_name = f"wasm-benchmark-{backend}.log" if publisher is not None else f"benchmark-{backend}.log"
            (self.args.output.parent / log_name).write_text(result.output)
            if result.returncode or not result_path.is_file():
                raise RuntimeError(f"{backend} Xiaoo benchmark failed: {result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}")
            data = json.loads(result_path.read_text())
            if len(data["rounds"]) != self.args.rounds * 2:
                raise AssertionError("worker did not execute every measured round")
            expected = self.args.warmup + self.args.count * self.args.rounds
            deadline = time.monotonic() + self.args.timeout
            while self._peers[0].accepted - before < expected and time.monotonic() < deadline:
                time.sleep(self.args.peer_poll_seconds)
            if self._peers[0].accepted - before != expected or self._peers[1].accepted:
                raise AssertionError("TCP peer counts differ from the measured allow/deny operations")
            if any(peer.failure for peer in self._peers):
                raise RuntimeError("TCP benchmark peer failed")
            data["peer_allowed_connections"] = self._peers[0].accepted - before
            data["operator_patches"] = [
                {"path": str(path), "content": path.read_text()} for path in patches
            ]
            data["policy_publication"] = publication
            data["xiaoo_log"] = log_name
            return data
        finally:
            try:
                if publisher is not None:
                    cleanup = publisher.cleanup()
                    if cleanup.has_failure():
                        raise RuntimeError(f"WASM benchmark cleanup failed: {cleanup}")
                else:
                    stopped = runtime.stop()
                    if stopped is not None and stopped.returncode:
                        raise RuntimeError(f"daemon cleanup failed: {stopped.output}")
            finally:
                provider.stop()

    def run(self) -> None:
        if os.geteuid() != 0:
            raise RuntimeError("run this BPF benchmark as root")
        self.args.output = self.args.output.resolve()
        self.args.output.parent.mkdir(parents=True, exist_ok=True)
        context = TestingContextSingleton(
            lock_path=Path("/run/lock/actrail-v2-regression.lock"),
            lock_timeout_seconds=self.args.lock_timeout, lock_poll_seconds=CASE_CONSTANTS.run_lock_poll_seconds,
            output=self._output,
        )
        root = Path(tempfile.mkdtemp(prefix=".benchmark-", dir=OUTPUT_DIR))
        try:
            for peer in self._peers:
                peer.start()
            measurements = {
                "scope": "IPv4 TCP connect+close through a real Xiaoo Bash tool",
                "policy_source": self.args.policy_source,
                "kernel": platform.release(), "architecture": platform.machine(),
                "count_per_decision_per_round": self.args.count, "rounds": self.args.rounds,
                "warmup_per_decision": self.args.warmup,
                "observation": "proc-lifecycle, net-transport, socket bpf-copy; TLS/stdio disabled",
                "audit": "network audit_enabled=false for both backends",
                "latency_scope": "Python socket creation, timeout setup, connect decision, close",
                "daemon_cpu_scope": "process utime+stime deltas from /proc/PID/stat during rounds",
                "limitations": "CPU ticks are quantized; Python/listener/scheduler costs are included; "
                    "provider and agent setup excluded from timed rounds; network control only; "
                    "backend order is sequential and configurable",
                "backend_order": self.args.backends, "measurements": [],
            }
            for backend in self.args.backends:
                print(f"Benchmarking {backend}: refreshing defaults", flush=True)
                measurements["measurements"].append(self._backend(root, backend))
                self.args.output.write_text(json.dumps(measurements, indent=2) + "\n")
            print(f"Measured results: {self.args.output}", flush=True)
        finally:
            try:
                for peer in self._peers:
                    peer.stop()
            finally:
                context.close()
                if self.args.keep_work:
                    print(f"Retained benchmark workspace: {root}")
                else:
                    shutil.rmtree(root)


