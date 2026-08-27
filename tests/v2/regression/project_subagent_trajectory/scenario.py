from __future__ import annotations

import secrets
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime

from .agent import ProjectSubagentAgent


class ProjectSubagentTrajectoryScenario:
    def __init__(
        self,
        runtime: ActrailRuntime,
        agent: ProjectSubagentAgent,
        repository: Path,
        launch_timeout_seconds: int,
        trace_random_bytes: int,
    ):
        self._runtime = runtime
        self._agent = agent
        self._repository = repository
        self._launch_timeout_seconds = launch_timeout_seconds
        self._trace_random_bytes = trace_random_bytes
        self.trace_name = f"PST_{self._agent.name.upper()}_{self._random_suffix()}"

    def run(self) -> None:
        result = self._runtime.run(
            [
                *self._runtime.control_command("launch"),
                "--name",
                self.trace_name,
                "--host-ebpf",
                "required",
                "--seccomp-notify",
                "auto",
                "--",
                *self._agent.launch_argv(self._repository, self._agent.prompt()),
            ],
            timeout_seconds=self._launch_timeout_seconds,
            environment=self._agent.environment,
            cwd=self._agent.runtime_cwd(self._repository),
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"{self._agent.name} launch exited with {result.returncode}: "
                f"{result.output[-6000:]}"
            )

    def _random_suffix(self) -> str:
        return secrets.token_hex(self._trace_random_bytes)
