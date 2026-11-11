"""Compare enabled and disabled continuous LSM checks with the same binaries."""

import json
import os
import platform
import shlex
import shutil
import socket
import statistics
import sys
import tempfile
import threading
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import TestOutput
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.runner import TestingContextSingleton
from ...services import WorkloadProvider
from ..paths import REPO


class DrainPeer:
    def __init__(self, timeout):
        self.timeout = timeout
        self.received = 0
        self.failure = None
        self.listener = socket.socket()
        self.listener.bind(("127.0.0.2", 0))
        self.listener.listen()
        self.listener.settimeout(timeout)
        self.port = self.listener.getsockname()[1]
        self.thread = threading.Thread(target=self.serve, daemon=True)
        self.thread.start()

    def serve(self):
        try:
            connection, _ = self.listener.accept()
            with connection:
                connection.settimeout(self.timeout)
                while data := connection.recv(65536):
                    if data != b"x" * len(data):
                        raise AssertionError("unexpected payload at TCP witness")
                    self.received += len(data)
        except BaseException as error:
            self.failure = str(error)

    def close(self):
        self.listener.close()
        self.thread.join(self.timeout)
        if self.thread.is_alive():
            raise RuntimeError("TCP witness did not finish")


class RetainedBenchmark:
    def __init__(self, args):
        self.args = args
        self.output = TestOutput(color_mode="never")
        binary = args.xiaoo or shutil.which("xiaoo")
        if not binary:
            raise RuntimeError("real Xiaoo is required")
        self.xiaoo = Path(binary).resolve()

    def patches(self, work, backend, port):
        enabled = backend != "off"
        capabilities = ["proc-lifecycle", "net-transport", "socket-plaintext-payload"]
        if enabled:
            capabilities += ["enforcement-file-permission-lsm", "enforcement-network-connect-lsm"]
        (work / "file.rules").write_text("" if backend == "empty" else f'bench-file allow open {work / "file"}\n')
        (work / "network.rules").write_text("" if backend == "empty" else f"bench-network allow connect 127.0.0.2:{port}\n")
        patch = work / "patch.toml"
        values = {
            "capture": {"profile_name": "retained-access-benchmark", "capabilities": capabilities,
                        "opportunistic_capabilities": []},
            "access_control": {"file": {"open": {"enabled": enabled, "backend": "lsm"},
                                         "mkdir": {"enabled": False}, "rmdir": {"enabled": False}},
                               "command": {"exec": {"enabled": False}},
                               "network": {"connect": {"enabled": enabled, "backend": "lsm"}}},
            "enforcement": {"enabled": enabled, "rules_path": str(work / "file.rules"),
                            "audit_enabled": False},
            "network_control": {"enabled": enabled, "rules_path": str(work / "network.rules"),
                                "audit_enabled": False, "audit_default_allow": False},
            "seccomp_notify": {"enabled": False},
            "storage": {"retention": {"enabled": False}},
        }
        PatchBuilder().with_work_dir(work).with_runtime_timing().with_values(values).build().save(patch)
        return [Path(__file__).parents[1] / "configuration/patch_config.toml", patch]

    def measure(self, work, backend):
        work.mkdir()
        (work / "file").write_bytes(b"." * 4096)
        peer = DrainPeer(self.args.timeout)
        result_path = work / "measurements.json"
        command = shlex.join([
            sys.executable, str(Path(__file__).with_name("run.py")), "worker",
            "--backend", backend, "--file", str(work / "file"), "--port", str(peer.port),
            "--count", str(self.args.count), "--rounds", str(self.args.rounds),
            "--warmup", str(self.args.warmup), "--timeout", str(self.args.timeout),
            "--xiaoo", str(self.xiaoo), "--daemon-pid", str(work / "run/actraild.pid"),
            "--results", str(result_path),
        ])
        provider = WorkloadProvider(REPO, work, command, self.args.timeout)
        patches = self.patches(work, backend, peer.port)
        runtime = ActrailRuntime(REPO, self.args.bin_dir, self.args.command_timeout, self.output,
                                 work / "actraild.conf", operator_config_patches=patches)
        try:
            url = provider.start()
            config = work / "xiaoo.toml"
            config.write_text(
                '[llm]\nprovider="deepseek"\nmodel="deepseek-chat"\n'
                'api_key_env="ACTRAIL_RETAINED_KEY"\n'
                f'api_base={json.dumps(url)}\nmax_tokens=1024\ncontext_window=64000\n'
            )
            runtime.prepare()
            environment = os.environ.copy()
            for key in ("HTTP_PROXY", "HTTPS_PROXY", "ALL_PROXY", "http_proxy", "https_proxy", "all_proxy"):
                environment.pop(key, None)
            environment.update(ACTRAIL_RETAINED_KEY="local-test-key", NO_PROXY="*", no_proxy="*", PATH="/usr/bin")
            result = runtime.run(runtime.control_command(
                "launch", "--seccomp-notify", "disabled", "--name", f"retained-{backend}",
                "--", self.xiaoo, "--cli", "run", "--config", config, "--tools", "bash",
                "--max-turns", "3", "--debug", "--prompt", "Use Bash exactly once and report the result.",
            ), timeout_seconds=self.args.launch_timeout, environment=environment)
            if result.returncode or not result_path.is_file():
                raise AssertionError(f"real Xiaoo benchmark failed: {result.output[-6000:]}")
            document = json.loads(result_path.read_text())
            peer.thread.join(self.args.timeout)
            if peer.failure or peer.received != document["sent_bytes"]:
                raise AssertionError(f"network effects mismatch: {peer.received}, {peer.failure}")
            document.update(received_bytes=peer.received, patches=[path.read_text() for path in patches])
            return document
        finally:
            try:
                stopped = runtime.stop()
                if stopped is not None and stopped.returncode:
                    raise RuntimeError(f"daemon cleanup failed: {stopped.output}")
            finally:
                try:
                    provider.stop()
                finally:
                    peer.close()

    def run(self):
        self.args.output.parent.mkdir(parents=True, exist_ok=True)
        context = TestingContextSingleton(
            lock_path=Path("/run/lock/actrail-v2-regression.lock"),
            lock_timeout_seconds=self.args.timeout, lock_poll_seconds=0.1, output=self.output,
        )
        root = Path(tempfile.mkdtemp(prefix="actrail-retained-"))
        data = {"kernel": platform.release(), "architecture": platform.machine(),
                "scope": "same binaries, fresh defaults; control off vs LSM empty/explicit rules; identical observation; audit off",
                "timing": "pread/pwrite of 1 byte and sendall of 64 bytes; pre-opened FD and connected TCP",
                "measurements": []}
        try:
            for index, backend in enumerate(self.args.order):
                result = self.measure(root / f"{index}-{backend}", backend)
                data["measurements"].append(result)
                self.args.output.write_text(json.dumps(data, indent=2) + "\n")
                means = {operation: statistics.median(
                    row["mean_latency_us"] for row in result["rounds"] if row["decision"] == operation
                ) for operation in ("read", "write", "send")}
                print(f"{backend}: median of round means (us): {means}", flush=True)
        finally:
            context.close()
            shutil.rmtree(root)
