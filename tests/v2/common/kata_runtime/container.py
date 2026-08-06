from __future__ import annotations

import json
import re
import secrets
import time
from collections.abc import Mapping, Sequence
from dataclasses import dataclass
from enum import Enum

from .capabilities import CtrCapabilities
from .process import CommandResult, ManagedProcess, ProcessRunner
from .requirements import ContainerRequirements, KataCreateSpec, PreparePolicy


_RUN_LABEL = "io.actrail.test.run"
_CASE_LABEL = "io.actrail.test.case"
_SAFE_PREFIX = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")


@dataclass(frozen=True)
class _HostProcess:
    pid: int
    ppid: int
    arguments: str


class _State(Enum):
    NEW = "new"
    STARTED = "started"
    READY = "ready"
    CLOSED = "closed"


class KataTestContainer:
    """Owns one Kata task for exactly one context-manager lifetime."""

    def __init__(
        self,
        requirements: ContainerRequirements,
        runner: ProcessRunner,
        capabilities: CtrCapabilities,
        *,
        cleanup_timeout_seconds: float = 5,
    ) -> None:
        if not _SAFE_PREFIX.fullmatch(requirements.name_prefix):
            raise ValueError(
                "Kata container name prefix must contain only "
                f"letters, digits, dot, underscore or dash: {requirements.name_prefix}"
            )
        if cleanup_timeout_seconds <= 0:
            raise ValueError("Kata cleanup timeout must be positive")
        self.requirements = requirements
        self._runner = runner
        self._capabilities = capabilities
        self._run_id = secrets.token_hex(16)
        self._container_id = (
            f"actrail-v2-{requirements.name_prefix}-{self._run_id[:12]}"
        )
        self._spec: KataCreateSpec | None = None
        self._managed_processes: list[ManagedProcess] = []
        self._cleanup_timeout_seconds = cleanup_timeout_seconds
        self._containerd_resources_removed = False
        self._state = _State.NEW

    @property
    def container_id(self) -> str:
        if self._state in {_State.NEW, _State.CLOSED}:
            raise RuntimeError("Kata container is not started")
        return self._container_id

    def __enter__(self) -> KataTestContainer:
        return self.start()

    def __exit__(self, exc_type, exc_value, traceback) -> None:
        del exc_value, traceback
        try:
            self.close()
        except Exception:
            if exc_type is None:
                raise

    def start(self) -> KataTestContainer:
        if self._state is not _State.NEW:
            raise RuntimeError(f"Kata container cannot start from {self._state.value}")

        self.requirements.validate_static()
        image = self.requirements.image.ensure(self.requirements.prepare_policy)
        spec = self.requirements.create_spec(image)
        try:
            self._create(spec)
            first_check = self.requirements.validate_running(self)
            if first_check.ready:
                self._state = _State.READY
                return self
            if (
                not first_check.refreshable
                or self.requirements.prepare_policy
                is not PreparePolicy.REFRESH_INVALID
            ):
                raise RuntimeError(
                    "Kata running requirements are not satisfied: "
                    + first_check.reason
                )

            self._remove_owned_resources()
            refreshed_image = self.requirements.refresh(first_check.reason)
            refreshed_spec = self.requirements.create_spec(refreshed_image)
            self._create(refreshed_spec)
            second_check = self.requirements.validate_running(self)
            if not second_check.ready:
                raise RuntimeError(
                    "Kata running requirements are still not satisfied after one "
                    "refresh/recreate attempt: "
                    f"first={first_check.reason}; second={second_check.reason}"
                )
            self._state = _State.READY
            return self
        except Exception as error:
            diagnostic = self.diagnostics()
            self._cleanup_after_start_failure()
            raise RuntimeError(f"{error}\n{diagnostic}") from error

    def _create(self, spec: KataCreateSpec) -> None:
        self._spec = spec
        self._containerd_resources_removed = False
        self._run_checked(
            self._build_run_command(spec),
            "create Kata task",
            timeout=spec.ready_timeout_seconds,
        )
        self._state = _State.STARTED

    def is_running(self) -> bool:
        if self._spec is None or self._state is _State.CLOSED:
            return False
        result = self._runner.run(
            self._ctr("tasks", "list"),
            timeout=self._spec.ready_timeout_seconds,
        )
        if result.returncode != 0:
            raise _command_error("list Kata tasks", result)
        for line in result.stdout.splitlines()[1:]:
            columns = line.split()
            if len(columns) >= 3 and columns[0] == self._container_id:
                return columns[2].lower() == "running"
        return False

    def exec(
        self,
        command: Sequence[str],
        *,
        uid: int | None = None,
        gid: int | None = None,
        environment: Mapping[str, str] | None = None,
        timeout: float | None = None,
    ) -> CommandResult:
        exec_command = self._build_exec_command(
            command,
            uid=uid,
            gid=gid,
            environment=environment,
        )
        assert self._spec is not None
        return self._runner.run(
            exec_command,
            timeout=(
                self._spec.ready_timeout_seconds if timeout is None else timeout
            ),
        )

    def start_exec(
        self,
        command: Sequence[str],
        *,
        uid: int | None = None,
        gid: int | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> ManagedProcess:
        exec_command = self._build_exec_command(
            command,
            uid=uid,
            gid=gid,
            environment=environment,
        )
        process = self._runner.start(exec_command)
        self._managed_processes.append(process)
        return process

    def diagnostics(self) -> str:
        """Collect bounded host-side evidence for this Kata container."""

        if self._spec is None:
            return "Kata container has not been configured"
        sections: list[str] = []
        commands = (
            ("task list", self._ctr("tasks", "list")),
            ("container info", self._ctr("containers", "info", self._container_id)),
        )
        for name, command in commands:
            try:
                result = self._runner.run(
                    command,
                    timeout=self._spec.ready_timeout_seconds,
                )
                output = result.diagnostic or "<no output>"
                sections.append(f"== {name} rc={result.returncode} ==\n{output}")
            except Exception as error:
                sections.append(f"== {name} failed ==\n{error}")
        try:
            processes = self._owned_host_processes()
            process_lines = [
                f"{item.pid} {item.ppid} {item.arguments}"
                for item in processes.values()
            ]
            sections.append(
                "== owned shim/VMM candidates ==\n"
                + ("\n".join(process_lines) or "<none>")
            )
        except Exception as error:
            sections.append(f"== owned shim/VMM discovery failed ==\n{error}")
        return "\n".join(sections)

    def _build_exec_command(
        self,
        command: Sequence[str],
        *,
        uid: int | None,
        gid: int | None,
        environment: Mapping[str, str] | None,
    ) -> list[str]:
        if self._state not in {_State.STARTED, _State.READY} or self._spec is None:
            raise RuntimeError("Kata container is not available for exec")
        if not command or any(not str(value) for value in command):
            raise ValueError("Kata exec command must contain non-empty argv")
        if (uid is None) != (gid is None):
            raise ValueError("Kata exec UID and GID must be provided together")
        effective_uid = self._spec.uid if uid is None else uid
        effective_gid = self._spec.gid if gid is None else gid
        if effective_uid < 0 or effective_gid < 0:
            raise ValueError("Kata exec UID/GID must be non-negative")
        if not self.is_running():
            raise RuntimeError(f"Kata task is not running: {self._container_id}")

        exec_command = self._ctr(
            "tasks",
            "exec",
            "--exec-id",
            f"actrail-exec-{secrets.token_hex(8)}",
        )
        workload_command = [str(value) for value in command]
        if environment:
            workload_command = [
                "/usr/bin/env",
                *(f"{name}={value}" for name, value in sorted(environment.items())),
                *workload_command,
            ]
        if self._capabilities.exec_user:
            exec_command.extend(["--user", f"{effective_uid}:{effective_gid}"])
        else:
            workload_command = [
                "/usr/bin/setpriv",
                "--reuid",
                str(effective_uid),
                "--regid",
                str(effective_gid),
                "--clear-groups",
                "--",
                *workload_command,
            ]
        exec_command.extend([self._container_id, *workload_command])
        return exec_command

    def close(self) -> None:
        if self._state is _State.CLOSED:
            return
        if self._spec is None:
            self._state = _State.CLOSED
            return

        process_error = self._terminate_managed_processes()
        try:
            self._remove_owned_resources()
        except Exception as cleanup_error:
            if self._containerd_resources_removed:
                self._state = _State.CLOSED
            if process_error is not None:
                raise cleanup_error from process_error
            raise
        self._state = _State.CLOSED
        if process_error is not None:
            raise process_error

    def _terminate_managed_processes(self) -> Exception | None:
        first_error: Exception | None = None
        for process in reversed(self._managed_processes):
            try:
                if process.poll() is None:
                    process.terminate(grace_seconds=2)
            except Exception as error:
                if first_error is None:
                    first_error = error
        self._managed_processes.clear()
        return first_error

    def _remove_owned_resources(self) -> None:
        assert self._spec is not None

        labels = self._container_labels()
        if labels is None:
            if self._task_pid() is not None:
                raise RuntimeError(
                    "Kata task remains after its ownership metadata disappeared: "
                    f"{self._container_id}"
                )
            return
        if labels.get(_RUN_LABEL) != self._run_id:
            raise RuntimeError(
                "refusing to remove Kata container not owned by this run: "
                f"{self._container_id}"
            )
        owned_processes = self._owned_host_processes()

        task_result = self._runner.run(
            self._ctr("tasks", "rm", "-f", self._container_id),
            timeout=self._spec.ready_timeout_seconds,
        )
        if task_result.returncode != 0 and not _is_missing(task_result):
            raise _command_error("remove Kata task", task_result)
        container_result = self._runner.run(
            self._ctr("containers", "rm", self._container_id),
            timeout=self._spec.ready_timeout_seconds,
        )
        if container_result.returncode != 0 and not _is_missing(container_result):
            raise _command_error("remove Kata container", container_result)
        self._verify_containerd_resources_removed()
        self._containerd_resources_removed = True
        self._wait_for_host_processes_exit(owned_processes)

    def _verify_containerd_resources_removed(self) -> None:
        if self._task_pid() is not None:
            raise RuntimeError(
                f"Kata task leaked after cleanup: {self._container_id}"
            )
        labels = self._container_labels()
        if labels is not None:
            raise RuntimeError(
                f"Kata container leaked after cleanup: {self._container_id}"
            )

    def _task_pid(self) -> int | None:
        assert self._spec is not None
        result = self._runner.run(
            self._ctr("tasks", "list"),
            timeout=self._spec.ready_timeout_seconds,
        )
        if result.returncode != 0:
            raise _command_error("list Kata tasks", result)
        for line in result.stdout.splitlines()[1:]:
            columns = line.split()
            if len(columns) < 2 or columns[0] != self._container_id:
                continue
            try:
                return int(columns[1])
            except ValueError as error:
                raise RuntimeError(
                    f"ctr returned an invalid task PID for {self._container_id}: "
                    f"{columns[1]}"
                ) from error
        return None

    def _owned_host_processes(self) -> dict[int, _HostProcess]:
        task_pid = self._task_pid()
        processes = self._host_processes()
        owned_pids = {
            process.pid
            for process in processes.values()
            if process.pid == task_pid or self._container_id in process.arguments
        }
        changed = True
        while changed:
            changed = False
            for process in processes.values():
                if process.ppid in owned_pids and process.pid not in owned_pids:
                    owned_pids.add(process.pid)
                    changed = True
        return {
            pid: processes[pid]
            for pid in sorted(owned_pids)
            if pid in processes
        }

    def _host_processes(self) -> dict[int, _HostProcess]:
        assert self._spec is not None
        result = self._runner.run(
            ["ps", "-eo", "pid=,ppid=,args="],
            timeout=self._spec.ready_timeout_seconds,
        )
        if result.returncode != 0:
            raise _command_error("list Kata host processes", result)
        processes: dict[int, _HostProcess] = {}
        for line in result.stdout.splitlines():
            columns = line.strip().split(maxsplit=2)
            if len(columns) != 3:
                continue
            try:
                pid = int(columns[0])
                ppid = int(columns[1])
            except ValueError:
                continue
            processes[pid] = _HostProcess(pid, ppid, columns[2])
        return processes

    def _wait_for_host_processes_exit(
        self,
        owned_processes: dict[int, _HostProcess],
    ) -> None:
        if not owned_processes:
            return
        deadline = time.monotonic() + self._cleanup_timeout_seconds
        remaining = owned_processes
        while remaining:
            current = self._host_processes()
            remaining = {
                pid: process
                for pid, process in owned_processes.items()
                if pid in current
                and current[pid].arguments == process.arguments
            }
            if not remaining:
                return
            if time.monotonic() >= deadline:
                details = "; ".join(
                    f"pid={item.pid} ppid={item.ppid} args={item.arguments}"
                    for item in remaining.values()
                )
                raise RuntimeError(
                    f"owned Kata host process leak for {self._container_id}: {details}"
                )
            time.sleep(min(0.1, max(0, deadline - time.monotonic())))

    def _build_run_command(self, spec: KataCreateSpec) -> list[str]:
        command = self._ctr("run", "-d", "--runtime", spec.runtime)
        if spec.runtime_config is not None:
            if not self._capabilities.runtime_config_path:
                raise RuntimeError("ctr run does not support --runtime-config-path")
            command.extend(["--runtime-config-path", str(spec.runtime_config)])

        labels = dict(spec.labels)
        labels[_RUN_LABEL] = self._run_id
        labels[_CASE_LABEL] = self.requirements.name_prefix
        for name, value in sorted(labels.items()):
            command.extend(["--label", f"{name}={value}"])
        for mount in spec.mounts:
            access = "ro" if mount.read_only else "rw"
            command.extend(
                [
                    "--mount",
                    "type=bind,"
                    f"src={mount.source},dst={mount.target},options=rbind:{access}",
                ]
            )
        for name, value in sorted(spec.environment):
            command.extend(["--env", f"{name}={value}"])

        workload_command = list(spec.command)
        if self._capabilities.run_user:
            command.extend(["--user", f"{spec.uid}:{spec.gid}"])
        else:
            workload_command = [
                "/usr/bin/setpriv",
                "--reuid",
                str(spec.uid),
                "--regid",
                str(spec.gid),
                "--clear-groups",
                "--",
                *workload_command,
            ]
        command.extend(
            [
                spec.image.reference,
                self._container_id,
                *workload_command,
            ]
        )
        return command

    def _container_labels(self) -> dict[str, str] | None:
        assert self._spec is not None
        result = self._runner.run(
            self._ctr("containers", "info", self._container_id),
            timeout=self._spec.ready_timeout_seconds,
        )
        if result.returncode != 0:
            if _is_missing(result):
                return None
            raise _command_error("inspect Kata container", result)
        try:
            payload = json.loads(result.stdout)
            labels = payload.get("Labels") or {}
            if not isinstance(labels, dict):
                raise TypeError("Labels is not an object")
            return {str(name): str(value) for name, value in labels.items()}
        except (json.JSONDecodeError, TypeError) as error:
            raise RuntimeError(
                f"ctr returned invalid container info for {self._container_id}"
            ) from error

    def _cleanup_after_start_failure(self) -> None:
        try:
            self.close()
        except Exception:
            # Preserve the original create/validation failure. The ownership
            # check in close still prevents deletion of a foreign resource.
            pass

    def _ctr(self, *arguments: str) -> list[str]:
        if self._spec is None:
            raise RuntimeError("Kata create spec has not been resolved")
        return ["ctr", "-n", self._spec.namespace, *arguments]

    def _run_checked(
        self,
        command: Sequence[str],
        operation: str,
        *,
        timeout: float,
    ) -> CommandResult:
        result = self._runner.run(command, timeout=timeout)
        if result.returncode != 0:
            raise _command_error(operation, result)
        return result


def _is_missing(result: CommandResult) -> bool:
    diagnostic = result.diagnostic.lower()
    return "not found" in diagnostic or "does not exist" in diagnostic


def _command_error(operation: str, result: CommandResult) -> RuntimeError:
    diagnostic = result.diagnostic or "no diagnostic output"
    return RuntimeError(
        f"failed to {operation} exit={result.returncode}: {diagnostic}"
    )
