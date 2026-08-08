from __future__ import annotations

import hashlib
import os
import shutil
import signal
import sqlite3
import subprocess
import sys
import tempfile
import time
from pathlib import Path
from typing import Protocol

from tests.v2.common.runner import TestingContextSingleton

from .config import ContainerAutoConfig
from .cases import (
    EbpfOffNotifyOffCase,
    EbpfOffNotifyOnCase,
    EbpfOnNotifyOffCase,
    EbpfOnNotifyOnCase,
)


REPO = Path(__file__).resolve().parents[5]
CONTAINER_MANAGER_DIR = (
    REPO
    / "tests"
    / "v2"
    / "common"
    / "test_suites"
    / "container-manager"
)
sys.path.insert(0, str(CONTAINER_MANAGER_DIR))

from container import TestContainer  # noqa: E402
from image import ContainerImage  # noqa: E402
from request import ContainerRequest  # noqa: E402


class MatrixCaseDefinition(Protocol):
    suffix: str
    expected_profile: str
    expected_host: str
    expected_notify: str
    custom_seccomp: bool
    host_ebpf_enabled: bool
    progress_step: str
    progress_message: str


class ContainerAutoScenario:
    def __init__(
        self,
        config: ContainerAutoConfig,
        test_context: TestingContextSingleton,
    ):
        self._config = config
        self._test_context = test_context
        self._case_dir = Path(__file__).resolve().parent
        self._runtime = Path(
            tempfile.mkdtemp(prefix="actrail-container-auto-v2.", dir="/tmp")
        )
        self._run_id = f"{int(time.time())}-{os.getpid()}"
        self._operator_template = self._case_dir / "operator.conf"
        self._operator_config = self._runtime / "container-auto.conf"
        self._database = self._runtime / "data/actrail.sqlite"
        self._daemon_log = self._runtime / "log/actraild.stderr"
        self._seccomp_profile = self._case_dir / "seccomp/actrail-notify.json"
        self._actraild = config.bin_dir.resolve() / "actraild"
        self._actrailctl = config.bin_dir.resolve() / "actrailctl"
        self._probe = (
            config.bin_dir.resolve() / "libactrail_tls_payload_probe_sync.so"
        )
        self._daemon: subprocess.Popen[str] | None = None
        self._daemon_ebpf_enabled: bool | None = None
        self._containers: list[TestContainer] = []
        self._execs: list[subprocess.Popen[str]] = []
        self._image: ContainerImage | None = None

    def run(self) -> None:
        try:
            self._prepare_runtime()
            self._prepare_image()
            EbpfOnNotifyOnCase().run(self)
            EbpfOnNotifyOffCase().run(self)
            EbpfOffNotifyOnCase().run(self)
            EbpfOffNotifyOffCase().run(self)

            self._test_context.report_progress(
                "peer_isolation",
                "checking cross-container control and TLS peer isolation",
            )
            self._ensure_host_ebpf(False)
            self._verify_peer_isolation()

            self._test_context.report_progress(
                "required_permissions",
                "checking required permission failures are explicit",
            )
            self._verify_required_permission_guards()

            self._test_context.report_progress(
                "daemon_restore",
                "restoring automatic host eBPF selection",
            )
            self._ensure_host_ebpf(True)
        finally:
            self._cleanup()

    def _prepare_runtime(self) -> None:
        for relative in (
            "run",
            "data/export",
            "log",
            "etc/actrail/plugins/otel-jsonl",
        ):
            (self._runtime / relative).mkdir(parents=True, exist_ok=True)
        plugin_source = self._config.repo / "examples/plugins/builtin/otel-jsonl"
        plugin_target = self._runtime / "etc/actrail/plugins/otel-jsonl"
        for name in (
            "otel-jsonl.plugin.toml",
            "otel-jsonl.config.v1.schema.json",
        ):
            shutil.copy2(plugin_source / name, plugin_target / name)
        plugin_config = (plugin_source / "otel-jsonl.config.toml").read_text(
            encoding="utf-8"
        )
        (plugin_target / "otel-jsonl.config.toml").write_text(
            plugin_config.replace("/var/lib/actrail", str(self._runtime / "data")),
            encoding="utf-8",
        )

    def _prepare_image(self) -> None:
        dockerfile = self._case_dir / "Dockerfile"
        digest = hashlib.sha256()
        digest.update(self._config.base_image.encode("utf-8"))
        for source in (dockerfile, self._actrailctl, self._probe):
            digest.update(source.name.encode("utf-8"))
            digest.update(source.read_bytes())
        version = digest.hexdigest()[:16]
        context = self._runtime / "image"
        context.mkdir()
        shutil.copy2(self._actrailctl, context / "actrailctl")
        shutil.copy2(
            self._probe,
            context / "libactrail_tls_payload_probe_sync.so",
        )
        build = ContainerImage(
            image_name="actrail/container-auto-v2",
            version=version,
            dockerfile_path=dockerfile,
            build_context=context,
            build_args={"BASE_IMAGE": self._config.base_image},
            force_rebuild=self._config.rebuild_image,
        )
        reference = build.ensure()
        self._image = ContainerImage(build.image_name, build.version)
        self._test_context.report_progress(
            "container_image",
            f"using content-addressed image {reference}",
        )

    def _render_config(self, ebpf_enabled: str) -> None:
        rendered = self._operator_template.read_text(encoding="utf-8")
        rendered = rendered.replace("@RUNTIME_DIR@", str(self._runtime))
        rendered = rendered.replace("@EBPF_ENABLED@", ebpf_enabled)
        if "@RUNTIME_DIR@" in rendered or "@EBPF_ENABLED@" in rendered:
            raise RuntimeError("container-auto operator template is unresolved")
        self._operator_config.write_text(rendered, encoding="utf-8")

    def _start_daemon(self, ebpf_enabled: str) -> None:
        self._stop_daemon()
        self._render_config(ebpf_enabled)
        for socket_name in ("control.sock", "tls-sync.sock", "actraild.pid"):
            try:
                (self._runtime / "run" / socket_name).unlink()
            except FileNotFoundError:
                pass
        daemon_log = self._daemon_log.open("a", encoding="utf-8")
        self._daemon = subprocess.Popen(
            [str(self._actraild), "--config", str(self._operator_config), "run"],
            cwd=self._config.repo,
            text=True,
            stdout=daemon_log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
        daemon_log.close()
        deadline = time.monotonic() + 30
        while time.monotonic() < deadline:
            if self._daemon.poll() is not None:
                raise RuntimeError(
                    "actraild exited during startup:\n" + self._daemon_log_tail()
                )
            if (self._runtime / "run/control.sock").is_socket() and (
                self._runtime / "run/tls-sync.sock"
            ).is_socket():
                return
            time.sleep(0.1)
        raise RuntimeError("isolated container-auto daemon sockets did not appear")

    def _require_doctor_ebpf(self, expected: bool) -> None:
        completed = self._host_command(
            [
                str(self._actrailctl),
                "--config",
                str(self._operator_config),
                "doctor",
            ]
        )
        present = "ebpf" in completed.stdout
        if present != expected:
            raise RuntimeError(
                f"doctor eBPF state mismatch expected={expected}: {completed.stdout}"
            )

    def _ensure_host_ebpf(self, enabled: bool) -> None:
        if self._daemon_ebpf_enabled == enabled and self._daemon is not None:
            return
        self._start_daemon("\"auto\"" if enabled else "false")
        self._require_doctor_ebpf(enabled)
        self._daemon_ebpf_enabled = enabled

    def run_matrix_case(self, case: MatrixCaseDefinition) -> None:
        self._ensure_host_ebpf(case.host_ebpf_enabled)
        self._test_context.report_progress(
            case.progress_step,
            case.progress_message,
        )
        before = self._latest_trace_id()
        container = self._new_container(
            f"actrail-auto-{case.suffix}-{self._run_id}",
            custom_seccomp=case.custom_seccomp,
        )
        try:
            process = container.exec(
                [
                    "/usr/local/bin/actrailctl",
                    "--config",
                    str(self._operator_config),
                    "launch",
                    "--host-ebpf",
                    "auto",
                    "--seccomp-notify",
                    "auto",
                    "--",
                    "/bin/sh",
                    "-c",
                    'curl -sS "$1" -o /dev/null && echo "$2-ok"',
                    "container-auto",
                    os.environ.get("TARGET_URL", "https://example.com/"),
                    case.suffix,
                ]
            )
            stdout, stderr = self._wait_exec(process)
            selection = (
                "deployment_permissions_selected="
                f"host_ebpf:{case.expected_host},"
                f"seccomp_notify:{case.expected_notify}"
            )
            if selection not in stdout:
                raise RuntimeError(
                    f"{case.suffix}: wrong permission selection\n{stdout}\n{stderr}"
                )
            trace_id = self._wait_for_trace_after(before)
            self._require_matrix_evidence(trace_id, case)
            self._require_unprivileged_container(container.request.name)
        finally:
            self._close_container(container)

    def _require_matrix_evidence(
        self,
        trace_id: int,
        case: MatrixCaseDefinition,
    ) -> None:
        deadline = time.monotonic() + 15
        last = ("", 0, 0, 0)
        while time.monotonic() < deadline:
            with sqlite3.connect(self._database) as connection:
                row = connection.execute(
                    "SELECT profile_name FROM traces WHERE trace_id = ?",
                    (trace_id,),
                ).fetchone()
                profile = "" if row is None else str(row[0])
                payloads = int(
                    connection.execute(
                        "SELECT COUNT(*) FROM payload_segments WHERE trace_id = ?",
                        (trace_id,),
                    ).fetchone()[0]
                )
                ebpf_events = int(
                    connection.execute(
                        "SELECT COUNT(*) FROM events "
                        "WHERE trace_id = ? AND collector = 'ebpf'",
                        (trace_id,),
                    ).fetchone()[0]
                )
                notify_events = int(
                    connection.execute(
                        "SELECT COUNT(*) FROM events "
                        "WHERE trace_id = ? AND collector = 'process-seccomp'",
                        (trace_id,),
                    ).fetchone()[0]
                )
            last = (profile, payloads, ebpf_events, notify_events)
            host_ok = ebpf_events > 0 if case.expected_host == "enabled" else ebpf_events == 0
            notify_ok = (
                notify_events > 0
                if case.expected_notify == "enabled"
                else notify_events == 0
            )
            if profile == case.expected_profile and payloads > 0 and host_ok and notify_ok:
                return
            time.sleep(0.2)
        raise RuntimeError(f"{case.suffix}: incomplete matrix evidence {last}")

    def _verify_peer_isolation(self) -> None:
        before = self._latest_trace_id()
        peer_a = self._new_container(f"actrail-peer-a-{self._run_id}", False)
        peer_b = self._new_container(f"actrail-peer-b-{self._run_id}", False)
        host_pid_peer = self._new_container(
            f"actrail-peer-host-pid-{self._run_id}",
            False,
            pid="host",
        )
        launch = peer_a.exec(
            [
                "/usr/local/bin/actrailctl",
                "--config",
                str(self._operator_config),
                "launch",
                "--host-ebpf",
                "disabled",
                "--seccomp-notify",
                "disabled",
                "--",
                "/bin/sleep",
                "120",
            ]
        )
        self._execs.append(launch)
        try:
            trace_id = self._wait_for_trace_after(before, active=True)
            for peer in (peer_b, host_pid_peer):
                stdout, _, _ = self._exec_result(
                    peer,
                    [
                        "actrailctl",
                        "--config",
                        str(self._operator_config),
                        "list-traces",
                    ],
                    check=True,
                )
                if f"trace-{trace_id} " in stdout:
                    raise RuntimeError(
                        f"{peer.request.name} can list another container trace"
                    )

            remove_command = [
                "actrailctl",
                "--config",
                str(self._operator_config),
                "track-remove",
                "--trace-id",
            ]
            foreign = self._exec_result(
                peer_b,
                [*remove_command, f"trace-{trace_id}"],
                check=False,
            )
            missing = self._exec_result(
                peer_b,
                [*remove_command, f"trace-{trace_id + 1_000_000}"],
                check=False,
            )
            if foreign[2] == 0 or "peer_identity" not in foreign[1]:
                raise RuntimeError("cross-container trace removal was not rejected")
            if missing[2] == 0 or foreign[1] != missing[1]:
                raise RuntimeError("track-remove disclosed foreign trace existence")

            host_remove = self._exec_result(
                host_pid_peer,
                [*remove_command, f"trace-{trace_id}"],
                check=False,
            )
            if host_remove[2] == 0:
                raise RuntimeError("host-PID container inherited host-root authority")

            self._require_seccomp_registration_rejected(peer_b, trace_id)
            self._require_tls_injection_rejected(peer_b, trace_id)
        finally:
            self._terminate_exec(launch)
            self._close_container(host_pid_peer)
            self._close_container(peer_b)
            self._close_container(peer_a)

    def _require_seccomp_registration_rejected(
        self,
        peer: TestContainer,
        trace_id: int,
    ) -> None:
        script = """
import array, os, socket, sys
fields = [b"register_seccomp_listener_v2", b"9001", sys.argv[1].encode(), str(os.getpid()).encode(), os.readlink("/proc/self/ns/pid").encode()]
frame = b"".join(str(len(field)).encode() + b"#" + field for field in fields)
listener_fd = os.open("/dev/null", os.O_RDONLY)
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(sys.argv[2])
client.sendmsg([frame], [(socket.SOL_SOCKET, socket.SCM_RIGHTS, array.array("i", [listener_fd]).tobytes())])
sys.stdout.buffer.write(client.recv(65536))
"""
        stdout, _, _ = self._exec_result(
            peer,
            [
                "python3",
                "-c",
                script,
                str(trace_id),
                str(self._runtime / "run/control.sock"),
            ],
            check=True,
        )
        if "peer_identity" not in stdout:
            raise RuntimeError("foreign seccomp listener registration was not rejected")

    def _require_tls_injection_rejected(
        self,
        peer: TestContainer,
        trace_id: int,
    ) -> None:
        offset = self._daemon_log.stat().st_size
        script = """
import os, socket, sys
client = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
client.connect(sys.argv[2])
start_ticks = open("/proc/self/stat", encoding="utf-8").read().split()[21]
pid_ns = os.readlink("/proc/self/ns/pid")
line = "v2\\tpayload\\t" + sys.argv[1] + "\\t" + str(os.getpid()) + "\\t" + start_ticks + "\\t" + pid_ns + "\\toutbound\\tpeer-e2e\\tinjection\\t1\\t1\\t6869\\n"
client.sendall(line.encode())
client.shutdown(socket.SHUT_WR)
client.settimeout(2)
try:
    while client.recv(4096):
        pass
except socket.timeout:
    pass
"""
        self._exec_result(
            peer,
            [
                "python3",
                "-c",
                script,
                str(trace_id),
                str(self._runtime / "run/tls-sync.sock"),
            ],
            check=True,
        )
        deadline = time.monotonic() + 10
        while time.monotonic() < deadline:
            with self._daemon_log.open("rb") as log:
                log.seek(offset)
                audit = log.read().decode("utf-8", errors="replace")
            if (
                "closed rejected TLS-sync peer" in audit
                and f"trace trace-{trace_id}" in audit
            ):
                break
            time.sleep(0.2)
        else:
            raise RuntimeError("foreign TLS payload injection lacked an audited rejection")
        with sqlite3.connect(self._database) as connection:
            forged = int(
                connection.execute(
                    "SELECT COUNT(*) FROM payload_segments "
                    "WHERE trace_id = ? AND library = 'peer-e2e' AND symbol = 'injection'",
                    (trace_id,),
                ).fetchone()[0]
            )
        if forged != 0:
            raise RuntimeError("foreign TLS payload reached another container trace")

    def _verify_required_permission_guards(self) -> None:
        container = self._new_container(
            f"actrail-required-{self._run_id}",
            custom_seccomp=False,
        )
        try:
            host = self._exec_result(
                container,
                [
                    "actrailctl",
                    "--config",
                    str(self._operator_config),
                    "launch",
                    "--host-ebpf",
                    "required",
                    "--seccomp-notify",
                    "disabled",
                    "--",
                    "/bin/true",
                ],
                check=False,
            )
            if host[2] == 0 or "host eBPF required" not in host[1]:
                raise RuntimeError("required host eBPF did not fail with a stable diagnostic")
            notify = self._exec_result(
                container,
                [
                    "actrailctl",
                    "--config",
                    str(self._operator_config),
                    "launch",
                    "--host-ebpf",
                    "disabled",
                    "--seccomp-notify",
                    "required",
                    "--",
                    "/bin/true",
                ],
                check=False,
            )
            if notify[2] == 0 or "seccomp-notify required" not in notify[1]:
                raise RuntimeError(
                    "required seccomp-notify did not fail with a stable diagnostic"
                )
        finally:
            self._close_container(container)

    def _new_container(
        self,
        name: str,
        custom_seccomp: bool,
        *,
        pid: str | None = None,
    ) -> TestContainer:
        if self._image is None:
            raise RuntimeError("container image is not prepared")
        security = (
            (f"seccomp={self._seccomp_profile}",) if custom_seccomp else ()
        )
        container = TestContainer(
            ContainerRequest(
                image=self._image,
                name=name,
                labels=(f"io.actrail.e2e-run={self._run_id}",),
                volumes=(f"{self._runtime}:{self._runtime}:ro",),
                security_options=security,
                user="0:0",
                pid=pid,
                ready_timeout_seconds=30,
            )
        ).start()
        self._containers.append(container)
        return container

    def _exec_result(
        self,
        container: TestContainer,
        command: list[str],
        *,
        check: bool,
    ) -> tuple[str, str, int]:
        process = container.exec(command)
        stdout, stderr = self._wait_exec(process, check=check)
        return stdout, stderr, int(process.returncode or 0)

    def _wait_exec(
        self,
        process: subprocess.Popen[str],
        *,
        check: bool = True,
    ) -> tuple[str, str]:
        self._execs.append(process)
        try:
            stdout, stderr = process.communicate(timeout=180)
        except subprocess.TimeoutExpired as error:
            self._terminate_exec(process)
            raise RuntimeError("docker exec timed out") from error
        finally:
            if process in self._execs:
                self._execs.remove(process)
        if check and process.returncode != 0:
            raise RuntimeError(
                f"docker exec failed exit={process.returncode}\n{stdout}\n{stderr}"
            )
        return stdout, stderr

    def _wait_for_trace_after(self, previous: int, *, active: bool = False) -> int:
        deadline = time.monotonic() + 15
        while time.monotonic() < deadline:
            if self._database.exists():
                with sqlite3.connect(self._database) as connection:
                    condition = "AND lifecycle_state = 'active'" if active else ""
                    row = connection.execute(
                        "SELECT trace_id FROM traces WHERE trace_id > ? "
                        f"{condition} ORDER BY trace_id DESC LIMIT 1",
                        (previous,),
                    ).fetchone()
                if row is not None:
                    return int(row[0])
            time.sleep(0.1)
        raise RuntimeError(f"no new trace appeared after trace-{previous}")

    def _latest_trace_id(self) -> int:
        if not self._database.exists():
            return 0
        with sqlite3.connect(self._database) as connection:
            return int(
                connection.execute(
                    "SELECT COALESCE(MAX(trace_id), 0) FROM traces"
                ).fetchone()[0]
            )

    def _require_unprivileged_container(self, name: str) -> None:
        completed = self._host_command(
            [
                "docker",
                "inspect",
                "--format",
                "{{.HostConfig.Privileged}}|{{.HostConfig.PidMode}}|{{json .HostConfig.CapAdd}}",
                name,
            ]
        )
        if completed.stdout.strip() != "false||null":
            raise RuntimeError(f"container permissions are broader than expected: {completed.stdout}")

    def _host_command(self, command: list[str]) -> subprocess.CompletedProcess[str]:
        completed = subprocess.run(
            command,
            cwd=self._config.repo,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=60,
            check=False,
        )
        if completed.returncode != 0:
            raise RuntimeError(
                f"command failed exit={completed.returncode}: {' '.join(command)}\n"
                f"{completed.stdout}\n{completed.stderr}"
            )
        return completed

    def _close_container(self, container: TestContainer) -> None:
        try:
            container.close()
        finally:
            if container in self._containers:
                self._containers.remove(container)

    def _terminate_exec(self, process: subprocess.Popen[str]) -> None:
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                process.wait()
        if process in self._execs:
            self._execs.remove(process)

    def _stop_daemon(self) -> None:
        if self._daemon is None:
            return
        if self._daemon.poll() is None:
            try:
                os.killpg(self._daemon.pid, signal.SIGTERM)
            except ProcessLookupError:
                pass
            try:
                self._daemon.wait(timeout=5)
            except subprocess.TimeoutExpired:
                try:
                    os.killpg(self._daemon.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                self._daemon.wait()
        self._daemon = None
        self._daemon_ebpf_enabled = None

    def _daemon_log_tail(self) -> str:
        if not self._daemon_log.exists():
            return "daemon log is missing"
        return self._daemon_log.read_text(encoding="utf-8", errors="replace")[-8000:]

    def _cleanup(self) -> None:
        for process in list(self._execs):
            self._terminate_exec(process)
        for container in reversed(list(self._containers)):
            try:
                self._close_container(container)
            except Exception:
                pass
        self._stop_daemon()
        shutil.rmtree(self._runtime, ignore_errors=True)
