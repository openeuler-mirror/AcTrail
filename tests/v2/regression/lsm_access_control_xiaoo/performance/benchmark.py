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
from tests.v2.common.core import TestOutput, has_failure
from tests.v2.common.runner import TestingContextSingleton
from ..services import WorkloadProvider
from .paths import ENTRY, OUTPUT_DIR, REPO
from .services import Peer, WasmPublisher


class Benchmark:
    def __init__(self, args: argparse.Namespace):
        self.args = args
        xiaoo = args.xiaoo or shutil.which("xiaoo")
        if not xiaoo:
            raise RuntimeError("real Xiaoo binary is unavailable")
        self._xiaoo = Path(xiaoo).resolve()
        self._output = TestOutput(color_mode="never")
        self._peers = [Peer(args.peer_poll_seconds, args.backlog) for _ in range(2)]

    def _patch(self, work: Path, backend: str) -> Path:
        patch = work / "actraild.patch.toml"
        ActrailRuntime.write_isolated_operator_config_patch(
            patch, work, payload_tls_enabled=False, payload_stdio_enabled=False,
        )
        document = patch.read_text().replace(
            "[payload.tls]\n", '[payload.tls]\ncapture_backend = "bpf-copy"\n'
        )
        document += (
            '\n[payload.socket]\nenabled = true\ncapture_backend = "bpf-copy"\n'
            '\n[capture]\nprofile_name = "network-control-benchmark"\n'
            'capabilities = ["proc-lifecycle", "net-transport", "socket-plaintext-payload", '
            f'"enforcement-network-connect-{backend}"]\nopportunistic_capabilities = []\n'
            '\n[ebpf]\nenabled = true\n'
            f'\n[seccomp_notify]\nenabled = {str(backend == "seccomp").lower()}\n'
            '\n[process_seccomp]\nenabled = false\n'
            f'\n[access_control.network.connect]\nbackend = "{backend}"\n'
            '\n[access_control.backends.lsm]\ndeny_signal = "none"\naudit_default_allow = false\n'
            '\n[enforcement]\nenabled = false\n'
            '\n[command_control]\nenabled = false\n'
            '\n[network_control]\nenabled = true\n'
            f'rules_path = {json.dumps(str(work / "network.rules"))}\n'
            'default_decision = "allow"\naudit_enabled = false\naudit_default_allow = false\n'
            '\n[plugins.startup]\nenabled = false\n'
        )
        patch.write_text(document)
        (work / "network.rules").write_text("" if self.args.policy_source == "wasm" else (
            f'bench-allow allow connect 127.0.0.1:{self._peers[0].port}\n'
            f'bench-deny deny connect 127.0.0.1:{self._peers[1].port}\n'
        ))
        return patch

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
        runtime = ActrailRuntime(
            REPO, self.args.bin_dir, self.args.command_timeout, self._output,
            work / "actraild.conf", self._patch(work, backend),
        )
        publisher = WasmPublisher(self.args, work, self._output, [peer.port for peer in self._peers]) \
            if self.args.policy_source == "wasm" else None
        if publisher is not None:
            runtime = publisher.runtime
        before = self._peers[0].accepted
        try:
            url = provider.start()
            config = work / "xiaoo.toml"
            config.write_text(
                '[llm]\nprovider = "deepseek"\nmodel = "deepseek-chat"\n'
                'api_key_env = "ACTRAIL_NETWORK_BENCHMARK_KEY"\n'
                f'api_base = {json.dumps(url)}\n'
                'max_tokens = 128\ncontext_window = 32768\nreasoning_effort = "off"\n'
            )
            publication = publisher.prepare_and_publish() if publisher is not None else None
            if publisher is None:
                runtime.prepare()
            environment = os.environ.copy()
            for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
                environment.pop(key, None)
            environment.update(
                NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost",
                ACTRAIL_NETWORK_BENCHMARK_KEY="local-test-key", PATH="/usr/bin",
            )
            result = runtime.run(
                runtime.control_command(
                    "launch", "--seccomp-notify", "required" if backend == "seccomp" else "disabled",
                    "--name", f"network-benchmark-{backend}", "--", self._xiaoo,
                    "--cli", "run", "--config", config, "--tools", "bash", "--max-turns", "3",
                    "--debug", "--prompt", "Use the Bash tool exactly once and report the result.",
                ), timeout_seconds=self.args.launch_timeout, environment=environment,
            )
            log_name = f"wasm-benchmark-{backend}.log" if publisher is not None else f"benchmark-{backend}.log"
            (self.args.output.parent / log_name).write_text(result.output)
            if result.returncode or not result_path.is_file():
                raise RuntimeError(f"{backend} Xiaoo benchmark failed: {result.output[-4000:]}")
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
            data["operator_patch"] = (work / "actraild.patch.toml").read_text()
            data["policy_publication"] = publication
            data["xiaoo_log"] = log_name
            return data
        finally:
            try:
                if publisher is not None:
                    cleanup = publisher.cleanup()
                    if has_failure(cleanup):
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
            lock_timeout_seconds=self.args.lock_timeout, lock_poll_seconds=1,
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


