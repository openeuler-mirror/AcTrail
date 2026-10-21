#!/usr/bin/env python3
"""Real operations invoked by Xiaoo's Bash tool under the selected LSM policy."""

from __future__ import annotations

import argparse
import ctypes
import json
import os
import socket
import struct
import subprocess
import time
from pathlib import Path

if __package__:
    from .constants import CASE_CONSTANTS
else:
    from constants import CASE_CONSTANTS


class AccessWorkload:
    IPV6_SOCKADDR_BYTES = 24

    def __init__(self, arguments: argparse.Namespace):
        self._arguments = arguments

    def run(self) -> None:
        results = {"processes": self._processes()}
        results["daemon_fds"] = self._descriptors(
            int(self._arguments.daemon_pid.read_text(encoding="utf-8").strip())
        )
        for domain, operation in (
            ("file", self._file), ("command", self._command), ("network", self._network),
            ("mkdir", self._mkdir), ("rmdir", self._rmdir),
        ):
            attempts = self._arguments.denial_repetitions if domain in ("file", "command") else 1
            denials = 0
            for index in range(attempts):
                try:
                    results[domain] = {"allowed": True, "value": operation()}
                except OSError as error:
                    results[domain] = {"allowed": False, "errno": error.errno, "error": str(error)}
                    denials += 1
                if index + 1 < attempts:
                    time.sleep(self._arguments.denial_pause)
            results[domain].update(attempts=attempts, denials=denials)
        if self._arguments.path_probes is not None:
            results["path_probes"] = PathProbes(self._arguments.timeout).run(self._arguments.path_probes)
        self._arguments.results.write_text(json.dumps(results), encoding="utf-8")
        print("ACTRAIL_LSM_WORKLOAD_RESULTS=" + json.dumps(results))

    def _file(self) -> str:
        self._arguments.marker.write_text("ACTRAIL_LSM_FILE", encoding="utf-8")
        return "ACTRAIL_LSM_FILE"

    def _command(self) -> int:
        result = subprocess.run(
            [str(self._arguments.executable)], check=False, timeout=self._arguments.timeout
        )
        if result.returncode != 0:
            raise RuntimeError(f"dedicated ELF exited with {result.returncode}")
        return result.returncode

    def _mkdir(self) -> str:
        self._arguments.mkdir.mkdir()
        return str(self._arguments.mkdir)

    def _rmdir(self) -> str:
        self._arguments.rmdir.rmdir()
        return str(self._arguments.rmdir)

    def _network(self) -> str:
        expected = b"ACTRAIL_LSM_NETWORK"
        with self._connect() as peer:
            peer.sendall(expected)
            actual = b""
            while len(actual) < len(expected):
                part = peer.recv(len(expected) - len(actual))
                if not part:
                    break
                actual += part
        if actual != expected:
            raise RuntimeError(f"unexpected TCP witness response: {actual!r}")
        return actual.decode()

    def _connect(self) -> socket.socket:
        if not self._arguments.ipv6:
            return socket.create_connection((CASE_CONSTANTS.witness_host, self._arguments.port), self._arguments.timeout)
        # Exercise a real kernel connect with the 24-byte sockaddr_in6 form.
        address = ctypes.create_string_buffer(
            struct.pack("=H", socket.AF_INET6) + struct.pack("!H", self._arguments.port)
            + bytes(4) + socket.inet_pton(socket.AF_INET6, CASE_CONSTANTS.ipv6_loopback_host)
        )
        libc = ctypes.CDLL(None, use_errno=True)
        libc.connect.argtypes = [ctypes.c_int, ctypes.c_void_p, ctypes.c_uint]
        libc.connect.restype = ctypes.c_int
        peer = socket.socket(socket.AF_INET6, socket.SOCK_STREAM)
        try:
            if libc.connect(peer.fileno(), address, self.IPV6_SOCKADDR_BYTES) != 0:
                code = ctypes.get_errno()
                raise OSError(code, os.strerror(code))
            peer.settimeout(self._arguments.timeout)
            return peer
        except BaseException:
            peer.close()
            raise

    @staticmethod
    def _descriptors(pid: int) -> list[str]:
        descriptors = []
        for entry in (Path("/proc") / str(pid) / "fd").iterdir():
            try:
                target = os.readlink(entry)
            except FileNotFoundError:
                continue
            if "fanotify" in target or "seccomp" in target:
                descriptors.append(target)
        return descriptors

    def _processes(self) -> list[dict]:
        processes = []
        pid = os.getpid()
        while pid > 1:
            root = Path("/proc") / str(pid)
            status = dict(line.split(":", 1) for line in (root / "status").read_text().splitlines())
            executable = (root / "exe").resolve()
            threads = []
            for task in (root / "task").iterdir():
                try:
                    task_status = dict(
                        line.split(":", 1) for line in (task / "status").read_text().splitlines()
                    )
                except FileNotFoundError:
                    continue
                threads.append({"tid": int(task.name), "seccomp": int(task_status["Seccomp"])})
            processes.append({"pid": pid, "executable": str(executable),
                              "threads": threads, "listener_fds": self._descriptors(pid)})
            if executable == self._arguments.xiaoo:
                return processes
            pid = int(status["PPid"])
        raise RuntimeError("workload process ancestry did not contain the real Xiaoo executable")


class PathProbes:
    """Perform the requested filesystem operations inside the real agent's tool."""

    def __init__(self, timeout: float):
        self._timeout = timeout

    def run(self, document: Path) -> dict:
        results = {}
        for probe in json.loads(document.read_text(encoding="utf-8")):
            path = Path(probe["path"])
            try:
                if probe["operation"] == "open":
                    path.write_text("ACTRAIL_PATH_WRITE", encoding="utf-8")
                elif probe["operation"] in ("exec", "exec-fd"):
                    descriptor = os.open(path, os.O_PATH) if probe["operation"] == "exec-fd" else None
                    try:
                        executable = str(path) if descriptor is None else f"/proc/self/fd/{descriptor}"
                        completed = subprocess.run(
                            [executable], timeout=self._timeout, check=False,
                            pass_fds=() if descriptor is None else (descriptor,),
                        )
                        if completed.returncode != 0:
                            raise RuntimeError(f"path probe ELF exited with {completed.returncode}: {path}")
                    finally:
                        if descriptor is not None:
                            os.close(descriptor)
                elif probe["operation"] == "mkdir":
                    path.mkdir()
                elif probe["operation"] == "rmdir":
                    path.rmdir()
                elif probe["operation"] in (
                    "mkdir-outside-cwd", "rmdir-outside-cwd", "mkdir-outside-dirfd", "rmdir-outside-dirfd",
                ):
                    self._outside_root(probe)
                elif probe["operation"] in ("mkdir-magic-dirfd", "rmdir-magic-dirfd"):
                    descriptor = os.open(path.parent.parent, os.O_PATH | os.O_DIRECTORY)
                    try:
                        target = f"/proc/self/fd/{descriptor}/{path.parent.name}/{path.name}"
                        operation = os.mkdir if probe["operation"].startswith("mkdir-") else os.rmdir
                        operation(target)
                    finally:
                        os.close(descriptor)
                else:
                    raise ValueError(f"unknown path probe operation: {probe}")
                results[probe["name"]] = {"allowed": True}
            except OSError as error:
                results[probe["name"]] = {"allowed": False, "errno": error.errno, "error": str(error)}
        return results

    def _outside_root(self, probe: dict) -> None:
        reader, writer = os.pipe()
        try:
            pid = os.fork()
        except BaseException:
            os.close(reader)
            os.close(writer)
            raise
        if pid == 0:
            os.close(reader)
            try:
                result = self._chroot_operation(probe)
                os.write(writer, json.dumps(result).encode())
            finally:
                os._exit(0)
        os.close(writer)
        try:
            with os.fdopen(reader, "r", encoding="utf-8") as stream:
                result = json.load(stream)
        finally:
            _, status = os.waitpid(pid, 0)
        if status != 0 or result.get("stage") != "operation":
            raise RuntimeError(f"root-external path probe setup failed: status={status}, result={result}")
        if result.get("errno") is not None:
            raise OSError(result["errno"], result["error"])

    @staticmethod
    def _chroot_operation(probe: dict) -> dict:
        stage = "setup"
        try:
            path = Path(probe["path"])
            descriptor = os.open(path.parent, os.O_PATH | os.O_DIRECTORY)
            mode = probe["operation"].rsplit("-", 1)[1]
            if mode == "cwd":
                os.chdir(path.parent)
            os.chroot(probe["root"])
            if mode == "dirfd":
                os.chdir("/")
            operation = os.mkdir if probe["operation"].startswith("mkdir-") else os.rmdir
            stage = "operation"
            operation(path.name, **({"dir_fd": descriptor} if mode == "dirfd" else {}))
            return {"stage": stage, "errno": None}
        except OSError as error:
            return {"stage": stage, "errno": error.errno, "error": str(error)}
        except Exception as error:
            return {"stage": "setup", "error": str(error)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ("marker", "executable", "results", "xiaoo", "daemon-pid", "mkdir", "rmdir"):
        parser.add_argument("--" + name, type=Path, required=True)
    parser.add_argument("--port", type=int, required=True)
    parser.add_argument("--timeout", type=float, required=True)
    parser.add_argument("--ipv6", action="store_true", default=CASE_CONSTANTS.ipv6)
    parser.add_argument("--denial-repetitions", type=int, default=CASE_CONSTANTS.workload_denial_repetitions)
    parser.add_argument("--denial-pause", type=float, default=CASE_CONSTANTS.workload_denial_pause)
    parser.add_argument("--path-probes", type=Path)
    AccessWorkload(parser.parse_args()).run()
