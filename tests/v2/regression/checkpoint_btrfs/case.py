from __future__ import annotations

import http.server
import json
import os
import re
import shlex
import shutil
import signal
import socket
import ssl
import subprocess
import threading
import time
import urllib.error
import urllib.request
from pathlib import Path
from typing import Callable

from tests.v2.common.core import TestCase, TestCaseInputs, TestResult, TestStatus
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.runner import TestingContextSingleton
from .constants import CASE_CONSTANTS as C

CHAT_BODY = json.dumps({"model": "m", "messages": [{"role": "user", "content": "hi"}]})


class _LlmHandler(http.server.BaseHTTPRequestHandler):
    """Answers POST like an LLM chat API and GET like an ordinary endpoint."""

    def do_POST(self) -> None:
        self.rfile.read(int(self.headers["Content-Length"]))
        self._reply(json.dumps({"id": "c1", "object": "chat.completion",
                                "choices": [{"message": {"role": "assistant", "content": "ok"}}]}).encode())

    def do_GET(self) -> None:
        self._reply(b"ok")

    def _reply(self, body: bytes) -> None:
        self.send_response(200)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def log_message(self, *args) -> None:
        pass


def _quote(path: Path) -> str:
    return shlex.quote(str(path))


class CheckpointBtrfsCase(TestCase):
    """Launch Agents with a Btrfs scope on a loop device and save, restore and delete their files."""

    def __init__(self, inputs: TestCaseInputs):
        self._inputs = inputs
        self._library = self._backend_library()
        self._work = inputs.work_dir
        self._mnt = self._work / "mnt"
        self._project = self._mnt / "workspaces/project"
        self._file = self._project / "a.txt"
        self._config = self._work / "actraild.conf"
        self._scopes = self._work / "scopes.toml"
        self._mounted = False
        self._daemon_started = False
        self._processes: list[subprocess.Popen] = []
        self._llm: http.server.HTTPServer | None = None
        self._chat = self._health = ""
        self._first_agent: subprocess.Popen | None = None
        self._llm_agent: subprocess.Popen | None = None
        self._first_trace = self._llm_trace = ""

    def _binary(self, name: str) -> Path:
        directory = self._inputs.bin_dir
        return (directory if directory.is_absolute() else self._inputs.repo / directory) / name

    def _backend_library(self) -> Path:
        if configured := os.environ.get(C.library_env):
            return Path(configured).resolve()
        target = Path(os.environ.get("CARGO_TARGET_DIR") or "target")
        if not target.is_absolute():
            target = self._inputs.repo / target
        return (target / "release" / C.library_filename).resolve()

    def _run(self, *command, timeout: float = C.command_timeout_seconds) -> subprocess.CompletedProcess:
        completed = subprocess.run([str(part) for part in command], cwd=self._inputs.repo,
                                   capture_output=True, text=True, timeout=timeout)
        if completed.returncode:
            raise AssertionError(f"{' '.join(map(str, command))} exited with {completed.returncode}: "
                                 f"{completed.stderr or completed.stdout}")
        return completed

    def _ctl(self, *args) -> str:
        return self._run(self._binary("actrailctl"), "--config", self._config, *args).stdout

    def _launch(self, name: str, script: str, scopes: Path | None = None) -> list[str]:
        return [str(self._binary("actrailctl")), "--config", str(self._config), "launch", "--name", name,
                "--checkpoint", str(scopes or self._scopes), "--", "sh", "-c", script]

    def _start(self, command: list[str], log: Path | None = None) -> subprocess.Popen:
        output = log.open("w", encoding="utf-8") if log else subprocess.DEVNULL
        process = subprocess.Popen(command, cwd=self._inputs.repo, stdout=output, stderr=subprocess.STDOUT,
                                   start_new_session=True)
        self._processes.append(process)
        return process

    @staticmethod
    def _wait_agent(process: subprocess.Popen) -> None:
        if code := process.wait(timeout=C.launch_timeout_seconds):
            raise AssertionError(f"Agent launch exited with {code}")

    @staticmethod
    def _wait(what: str, ready: Callable[[], object], timeout: float = C.wait_timeout_seconds):
        deadline = time.monotonic() + timeout
        while not (value := ready()):
            if time.monotonic() > deadline:
                raise AssertionError(f"timed out waiting for {what}")
            time.sleep(C.poll_seconds)
        return value

    def _content(self) -> str:
        return self._file.read_text(encoding="utf-8").strip()

    def _expect(self, expected: str, what: str) -> None:
        if (actual := self._content()) != expected:
            raise AssertionError(f"{what}: a.txt is {actual!r}, expected {expected!r}")

    def _trace(self, name: str) -> str:
        pattern = re.compile(rf"(?<![\w-]){re.escape(name)}(?![\w-])")
        for line in self._ctl("list-traces").splitlines():
            if pattern.search(line) and (match := re.search(r"trace-\d+", line)):
                return match.group()
        return ""

    def _listed(self, trace: str, pattern: str) -> bool:
        return re.search(pattern, self._ctl("checkpoint", "list", trace), re.M) is not None

    @staticmethod
    def _operation(output: str) -> str:
        if not (match := re.search(r"operation-\d+", output)):
            raise AssertionError(f"no operation id in {output!r}")
        return match.group()

    def _succeeded(self, operation: str) -> None:
        def finished() -> bool:
            view = self._ctl("checkpoint", "operation", operation)
            if "phase=finished" in view and "outcome=succeeded" not in view:
                raise AssertionError(f"{operation} did not succeed: {view}")
            return "phase=finished outcome=succeeded" in view
        self._wait(f"{operation} to finish", finished)

    def _restore_waiting(self, trace: str, checkpoint: str) -> str:
        operation = self._operation(self._ctl("checkpoint", "restore", trace, checkpoint))
        if "phase=waiting_boundary" not in (view := self._ctl("checkpoint", "operation", operation)):
            raise AssertionError(f"the restore of {trace} did not wait for an LLM request: {view}")
        return operation

    @staticmethod
    def _scope(scope_id: str, root: Path) -> str:
        return (f"[[scopes]]\nid = {json.dumps(scope_id)}\nroot = {json.dumps(str(root))}\n"
                f"backend_instance = {json.dumps(C.backend_instance)}\n")

    def _missing(self) -> str | None:
        if missing := [tool for tool in C.required_tools if shutil.which(tool) is None]:
            return "missing tools: " + ", ".join(missing)
        if subprocess.run(["modprobe", "btrfs"], capture_output=True).returncode:
            return "the kernel cannot load Btrfs"
        return None

    def _setup(self) -> str:
        work = self._work
        for directory in ("run", "log", "data", "mnt"):
            (work / directory).mkdir(parents=True, exist_ok=True)
        (work / "alert-forwarding.json").write_text(json.dumps({
            "enabled": False, "all_categories": False, "categories": [],
        }), encoding="utf-8")
        image = work / "filesystem.img"
        with image.open("wb") as handle:
            handle.truncate(C.filesystem_bytes)
        self._run("mkfs.btrfs", "-q", image)
        self._run("mount", "-o", "loop", image, self._mnt)
        self._mounted = True
        (self._mnt / "workspaces").mkdir()
        (self._mnt / ".actrail").mkdir()
        self._run("btrfs", "subvolume", "create", self._project)
        self._file.write_text("one\n", encoding="utf-8")
        (work / "btrfs.json").write_text(json.dumps({
            "btrfs_path": shutil.which("btrfs"),
            "findmnt_path": shutil.which("findmnt"),
            "copy_path": shutil.which("cp"),
            "snapshot_root": str(self._mnt / ".actrail"),
            "allowed_workspace_roots": [str(self._mnt / "workspaces")],
        }, indent=2), encoding="utf-8")
        patch = PatchBuilder().with_work_dir(work).with_values({
            "supervision": {"startup_wait_ms": C.daemon_startup_wait_ms},
            "alert_forwarding": {"socket_path": str(work / "run/alert-proxy.sock"),
                                 "plugin_config_path": str(work / "alert-forwarding.json")},
            "sandbox_evidence": {"path": str(work / "data/sandbox-evidence.sqlite")},
            "sandbox_alerts": {"path": str(work / "data/sandbox-alerts.sqlite")},
            "payload": {"tls": {"capture_backend": "tls-sync"}},
            "checkpoint": {"enabled": True, "catalog_path": str(work / "data/checkpoint.db"), "backends": [{
                "name": C.backend_instance, "library": str(self._library),
                "config": str(work / "btrfs.json"),
            }]},
        }).build().save(work / "checkpoint.patch.toml")
        self._ctl("init", "--force", "--patch", patch)
        self._scopes.write_text(self._scope("project", self._project), encoding="utf-8")
        self._run(self._binary("actraild"), "--config", self._config, "start",
                  timeout=C.launch_timeout_seconds)
        self._daemon_started = True
        return "loop Btrfs filesystem, backend configuration and daemon ready"

    def _initial_checkpoint(self) -> str:
        file = _quote(self._file)
        script = (f"echo two > {file}; "
                  f"while [ ! -e {_quote(self._work / 'manual-save-done')} ]; do sleep {C.poll_seconds}; done; "
                  f"echo three > {file}")
        self._first_agent = self._start(self._launch("checkpoint-e2e", script))
        self._first_trace = self._wait("the Agent to start writing",
                                       lambda: self._content() == "two" and self._trace("checkpoint-e2e"))
        if not self._listed(self._first_trace, r"^checkpoint-0 .*restorable"):
            raise AssertionError("the initial checkpoint is missing")
        return "checkpoint-0 saved before the Agent ran"

    def _manual_save(self) -> str:
        self._succeeded(self._operation(self._ctl("checkpoint", "save", self._first_trace)))
        return "manual save while the Agent runs"

    def _delete(self) -> str:
        self._ctl("checkpoint", "delete", self._first_trace, "checkpoint-1")
        self._wait("checkpoint-1 to be released",
                   lambda: self._listed(self._first_trace, r"^checkpoint-1 .*release=released"))
        if len(snapshots := list((self._mnt / ".actrail/snapshots").glob("*/*/*"))) != 1:
            raise AssertionError(f"the deleted snapshot is still there: {snapshots}")
        return "delete releases the snapshot"

    def _trace_end(self) -> str:
        (self._work / "manual-save-done").touch()
        self._wait_agent(self._first_agent)
        self._expect("three", "after the Agent finished")
        self._wait("checkpoint-0 to be released at the end of the Trace",
                   lambda: self._listed(self._first_trace, r"^checkpoint-0 .*release=released"))
        if snapshots := list((self._mnt / ".actrail/snapshots").glob("*/*/*")):
            raise AssertionError(f"snapshots survived the Trace: {snapshots}")
        self._refused_restore(self._first_trace)
        return "the end of the Trace releases its snapshots and refuses restores"

    def _refused_restore(self, trace: str) -> None:
        restore = subprocess.run([str(self._binary("actrailctl")), "--config", str(self._config), "checkpoint",
                                  "restore", trace, "checkpoint-0"],
                                 cwd=self._inputs.repo, capture_output=True, text=True,
                                 timeout=C.command_timeout_seconds)
        if restore.returncode == 0:
            raise AssertionError(f"a restore of {trace} was accepted")

    def _connect_failure(self) -> str:
        outside = self._work / "outside"
        outside.mkdir()
        scopes = self._work / "bad-scopes.toml"
        scopes.write_text(self._scope("outside", outside), encoding="utf-8")
        ran = outside / "ran"
        launch = subprocess.run(self._launch("checkpoint-e2e-bad", f"touch {_quote(ran)}", scopes),
                                cwd=self._inputs.repo, capture_output=True, text=True, timeout=C.launch_timeout_seconds)
        if launch.returncode == 0:
            raise AssertionError("a launch with a scope outside allowed_workspace_roots succeeded")
        if "connect_failed" not in launch.stderr:
            raise AssertionError(f"unexpected launch error: {launch.stderr}")
        if ran.exists():
            raise AssertionError("the Agent ran although connecting failed")
        return "a scope the backend cannot honor fails the launch before the Agent runs"

    def _start_llm(self) -> None:
        cert, key = self._work / "cert.pem", self._work / "key.pem"
        self._run("openssl", "req", "-x509", "-newkey", "rsa:2048", "-nodes", "-keyout", key, "-out", cert,
                  "-days", "1", "-subj", "/CN=localhost")
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), _LlmHandler)
        context = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
        context.load_cert_chain(cert, key)
        server.socket = context.wrap_socket(server.socket, server_side=True)
        threading.Thread(target=server.serve_forever, daemon=True).start()
        self._llm = server
        url = f"https://127.0.0.1:{server.server_address[1]}"
        self._chat = (f"curl -sk --http1.1 {url}/v1/chat/completions -H 'Content-Type: application/json' "
                      f"-d {shlex.quote(CHAT_BODY)} >/dev/null")
        self._health = f"curl -sk --http1.1 {url}/health >/dev/null"

    def _llm_boundary(self) -> str:
        self._start_llm()
        self._file.write_text("four\n", encoding="utf-8")
        file = _quote(self._file)
        script = (f"echo five > {file}; {self._chat}; {self._chat}; echo six > {file}; {self._health}; "
                  f"{self._chat}; echo seven > {file}; "
                  f"while [ ! -e {_quote(self._work / 'llm-finish')} ]; do sleep 0.1; done")
        self._llm_agent = self._start(self._launch("checkpoint-e2e-llm", script))
        self._llm_trace = self._wait("the LLM Agent to finish its requests",
                                     lambda: self._content() == "seven" and self._trace("checkpoint-e2e-llm"),
                                     timeout=C.launch_timeout_seconds)
        operations = self._ctl("checkpoint", "operations", self._llm_trace)
        if (saves := operations.count("trigger=llm_boundary phase=finished outcome=succeeded")) != 3:
            raise AssertionError(f"expected 3 saves at 3 LLM requests, got {saves}: {operations}")
        return "every LLM request is saved before its response, including without file changes"

    def _forced_restore(self) -> str:
        for checkpoint, expected in (("checkpoint-1", "five"), ("checkpoint-2", "five"),
                                     ("checkpoint-3", "six")):
            operation = self._restore_waiting(self._llm_trace, checkpoint)
            self._ctl("checkpoint", "force", operation)
            self._succeeded(operation)
            self._expect(expected, f"forced restore to {checkpoint}")
        return "force starts a restore that waits for an LLM request"

    @staticmethod
    def _http(url: str, method: str = "GET") -> str:
        request = urllib.request.Request(url, method=method, data=b"" if method == "POST" else None)
        with urllib.request.urlopen(request, timeout=C.command_timeout_seconds) as reply:
            return reply.read().decode()

    def _web(self) -> str:
        with socket.socket() as probe:
            probe.bind(("127.0.0.1", 0))
            port = probe.getsockname()[1]
        self._start([str(self._binary("actrailweb")), "--config", str(self._config), "--addr", "127.0.0.1",
                     "--port", str(port)], self._work / "log/web.log")
        api = f"http://127.0.0.1:{port}/api/traces/{self._llm_trace.removeprefix('trace-')}/checkpoints"

        def listing() -> dict | None:
            try:
                return json.loads(self._http(api))
            except (urllib.error.URLError, ConnectionError):
                return None
        saves = [op for op in self._wait("Web to answer", listing)["operations"] if op["trigger"] == "llm_boundary"]
        if len(saves) != 3 or not all(op["llm_call_action_id"] for op in saves):
            raise AssertionError(f"Web did not match the LLM boundaries: {saves}")
        operation, checkpoint = saves[0]["operation_id"], saves[0]["created_checkpoint"]
        if '"kind":"llm.call"' not in self._http(f"{api}/navigation?operation={operation}"):
            raise AssertionError(f"Web found no ActionTree path to the LLM call of operation-{operation}")
        if '"accepted":true' not in self._http(f"{api}/delete?checkpoint={checkpoint}", "POST"):
            raise AssertionError(f"Web could not delete checkpoint-{checkpoint}")
        if not self._listed(self._llm_trace, rf"^checkpoint-{checkpoint} .*deleted"):
            raise AssertionError(f"checkpoint-{checkpoint} is not deleted")
        (self._work / "llm-finish").touch()
        self._wait_agent(self._llm_agent)
        return "Web matches all LLM boundaries to the ActionTree and runs checkpoint commands"

    def _running_restore(self) -> str:
        go, seen, file = self._work / "go", self._work / "seen", _quote(self._file)
        script = (f"echo eight > {file}; {self._chat}; echo nine > {file}; "
                  f"while [ ! -e {_quote(go)} ]; do sleep 0.1; done; {self._chat}; cat {file} > {_quote(seen)}")
        agent = self._start(self._launch("checkpoint-e2e-running", script))
        trace = self._wait("the running Agent to get past its first LLM request",
                           lambda: self._content() == "nine" and self._trace("checkpoint-e2e-running"))
        latest = re.findall(r"^checkpoint-\d+", self._ctl("checkpoint", "list", trace), re.M)[-1]
        operation = self._restore_waiting(trace, latest)
        go.touch()
        self._succeeded(operation)
        self._wait_agent(agent)
        if (read := seen.read_text(encoding="utf-8").strip()) != "eight":
            raise AssertionError(f"after the restore the Agent read {read!r}")
        return "a running Agent is restored at its next LLM request, before it gets the response"

    def _daemon_restart(self) -> str:
        self._run(self._binary("actraild"), "--config", self._config, "restart",
                  timeout=C.launch_timeout_seconds)
        if not self._listed(self._first_trace, r"^checkpoint-0 "):
            raise AssertionError("the history did not survive a daemon restart")
        self._refused_restore(self._first_trace)
        return "the history survives a daemon restart and restores stay refused"

    def _daemon_log(self) -> str:
        log = self._work / "log/actraild.log"
        return log.read_text(encoding="utf-8", errors="replace")[-2000:] if log.exists() else ""

    def _teardown(self) -> str | None:
        failures: list[str] = []
        if self._llm is not None:
            self._llm.shutdown()
            self._llm.server_close()
        for process in self._processes:
            # The launch wrapper may have exited while an Agent is still alive or stopped.
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            try:
                process.wait(timeout=C.stop_timeout_seconds)
            except subprocess.TimeoutExpired as error:
                failures.append(str(error))
        if self._daemon_started:
            try:
                self._run(self._binary("actraild"), "--config", self._config, "stop")
            except (AssertionError, subprocess.TimeoutExpired) as error:
                failures.append(str(error))
        if self._mounted:
            try:
                self._run("umount", self._mnt)
            except (AssertionError, subprocess.TimeoutExpired) as error:
                failures.append(str(error))
        return "\n".join(failures) or None

    def run(self, context: TestingContextSingleton) -> TestResult:
        if not self._library.is_file():
            return TestResult(TestStatus.FAILED, f"Btrfs backend library not found: {self._library}")
        if missing := self._missing():
            return TestResult(TestStatus.SKIPPED, missing)
        steps = [
            ("setup", self._setup),
            ("initial_checkpoint", self._initial_checkpoint),
            ("manual_save", self._manual_save),
            ("delete", self._delete),
            ("trace_end", self._trace_end),
            ("connect_failure", self._connect_failure),
            ("llm_boundary", self._llm_boundary),
            ("forced_restore", self._forced_restore),
            ("web", self._web),
            ("running_restore", self._running_restore),
            ("daemon_restart", self._daemon_restart),
        ]
        results: dict[str, TestResult] = {}
        try:
            for name, step in steps:
                context.report_progress(name)
                try:
                    results[name] = TestResult(TestStatus.PASSED, step())
                except Exception as error:
                    results[name] = TestResult(TestStatus.FAILED, f"{error}\n{self._daemon_log()}")
                    break
        finally:
            if failure := self._teardown():
                results["cleanup"] = TestResult(TestStatus.FAILED, failure)
        return TestResult(TestStatus.COMPOSITE, "checkpoint with the Btrfs backend", results)
