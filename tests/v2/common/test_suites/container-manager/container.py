from __future__ import annotations

import os
import subprocess
import time
from collections.abc import Mapping, Sequence

from request import ContainerRequest


class TestContainer:
    """Owns one long-lived container and runs short tasks through docker exec."""

    def __init__(self, request: ContainerRequest):
        self.request = request
        self._container_id: str | None = None

    @property
    def container_id(self) -> str:
        if self._container_id is None:
            raise RuntimeError(f"container is not started: {self.request.name}")
        return self._container_id

    def __enter__(self) -> TestContainer:
        return self.start()

    def __exit__(self, exc_type, exc_val, exc_tb) -> None:
        try:
            self.close()
        except Exception:
            if exc_type is None:
                raise

    def start(self) -> TestContainer:
        existing = self._inspect_id()
        if existing is not None:
            if not self.request.force_overwrite:
                raise RuntimeError(
                    f"container already exists and is not owned by this context: "
                    f"{self.request.name}"
                )
            self._remove()

        image = self.request.image.ensure()
        command = ["docker", "run", "-d", "--name", self.request.name]
        for label in self.request.labels:
            command.extend(["--label", label])
        if self.request.user is not None:
            command.extend(["--user", self.request.user])
        if self.request.network is not None:
            command.extend(["--network", self.request.network])
        if self.request.pid is not None:
            command.extend(["--pid", self.request.pid])
        for security_option in self.request.security_options:
            command.extend(["--security-opt", security_option])
        for volume in self.request.volumes:
            command.extend(["--volume", volume])
        command.extend([image, *self.request.command])

        try:
            self._container_id = self._run(command, "start container").strip()
            self._wait_until_running()
            return self
        except Exception:
            if self._inspect_id() is not None:
                self._remove()
            self._container_id = None
            raise

    def exec(
        self,
        command: Sequence[str],
        *,
        environment: Mapping[str, str] | None = None,
    ) -> subprocess.Popen[str]:
        self._require_running_pid()
        docker_command = ["docker", "exec"]
        for name, value in sorted((environment or {}).items()):
            docker_command.extend(["--env", f"{name}={value}"])
        docker_command.extend([self.request.name, *command])
        return subprocess.Popen(
            docker_command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            start_new_session=True,
        )

    def pid_namespace(self) -> str:
        host_pid = self._require_running_pid()
        return os.readlink(f"/proc/{host_pid}/ns/pid")

    def close(self) -> None:
        if not self.request.dismiss_on_exit:
            return
        if self._container_id is None and self._inspect_id() is None:
            return
        self._remove()
        self._container_id = None

    def _wait_until_running(self) -> None:
        deadline = time.monotonic() + self.request.ready_timeout_seconds
        last_state = "missing"
        while time.monotonic() < deadline:
            state = self._inspect_state()
            if state is not None:
                status, pid = state
                last_state = f"status={status} pid={pid}"
                if status == "running" and pid > 0:
                    return
                if status in {"dead", "exited"}:
                    break
            time.sleep(0.05)
        raise RuntimeError(
            f"container did not become ready: {self.request.name} {last_state}"
        )

    def _require_running_pid(self) -> int:
        state = self._inspect_state()
        if state is None:
            raise RuntimeError(f"container does not exist: {self.request.name}")
        status, pid = state
        if status != "running" or pid <= 0:
            raise RuntimeError(
                f"container is not running: {self.request.name} "
                f"status={status} pid={pid}"
            )
        return pid

    def _inspect_id(self) -> str | None:
        result = subprocess.run(
            ["docker", "inspect", "--format", "{{.Id}}", self.request.name],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode == 0:
            return result.stdout.strip()
        if "no such object" in result.stderr.lower():
            return None
        raise RuntimeError(
            f"failed to inspect container {self.request.name}: "
            f"{result.stderr.strip()}"
        )

    def _inspect_state(self) -> tuple[str, int] | None:
        result = subprocess.run(
            [
                "docker",
                "inspect",
                "--format",
                "{{.State.Status}} {{.State.Pid}}",
                self.request.name,
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            if "no such object" in result.stderr.lower():
                return None
            raise RuntimeError(
                f"failed to inspect container {self.request.name}: "
                f"{result.stderr.strip()}"
            )
        status, raw_pid = result.stdout.strip().split(maxsplit=1)
        return status, int(raw_pid)

    def _remove(self) -> None:
        self._run(
            ["docker", "rm", "-f", self.request.name],
            "remove container",
        )

    @staticmethod
    def _run(command: list[str], operation: str) -> str:
        result = subprocess.run(
            command,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            check=False,
        )
        if result.returncode != 0:
            diagnostic = (result.stderr or result.stdout).strip()
            raise RuntimeError(
                f"failed to {operation} exit={result.returncode}: {diagnostic}"
            )
        return result.stdout
