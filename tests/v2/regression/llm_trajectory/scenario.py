from __future__ import annotations

import json
import os
import re
import secrets
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult


@dataclass(frozen=True)
class FixtureRepository:
    path: Path
    commit_id: str

    @classmethod
    def create(cls, runtime: ActrailRuntime, work_dir: Path) -> "FixtureRepository":
        path = work_dir / "trajectory-fixture"
        path.mkdir(parents=True, exist_ok=False)
        (path / "README.md").write_text(
            "# LLM trajectory fixture\n\n"
            f"fixture_nonce={secrets.token_hex(12)}\n",
            encoding="utf-8",
        )
        commands: tuple[tuple[str, ...], ...] = (
            ("git", "-C", str(path), "init", "--quiet"),
            ("git", "-C", str(path), "config", "user.name", "AcTrail E2E"),
            (
                "git",
                "-C",
                str(path),
                "config",
                "user.email",
                "actrail-e2e@example.invalid",
            ),
            ("git", "-C", str(path), "add", "README.md"),
            (
                "git",
                "-C",
                str(path),
                "commit",
                "--quiet",
                "-m",
                "fixture: trajectory commit",
            ),
        )
        for command in commands:
            runtime.run_checked(list(command), echo=False)
        commit_id = runtime.run_checked(
            ["git", "-C", path, "rev-parse", "HEAD"],
            echo=False,
        ).stdout.strip()
        if re.fullmatch(r"[0-9a-f]{40}", commit_id) is None:
            raise RuntimeError(f"fixture produced invalid commit id: {commit_id!r}")
        return cls(path=path, commit_id=commit_id)


class OpenCodeSubagentScenario:
    _TRACE_PATTERN = re.compile(r"trace trace-(\d+) entered Active")

    def __init__(
        self,
        runtime: ActrailRuntime,
        binary: Path,
        environment: dict[str, str],
        fixture: FixtureRepository,
        launch_timeout_seconds: int,
    ):
        self._runtime = runtime
        self._binary = binary
        self._environment = environment
        self._fixture = fixture
        self._launch_timeout_seconds = launch_timeout_seconds
        self.trace_name = f"LLM_TRAJECTORY_{secrets.token_hex(8)}"
        self.task_marker = f"SUBTASK_{secrets.token_hex(8)}"
        self.answer_marker = f"RESULT_{secrets.token_hex(8)}"

    def run(self) -> tuple[int, CommandResult]:
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
                self._binary,
                "run",
                "--pure",
                "--auto",
                "--format",
                "json",
                "--dir",
                self._fixture.path,
                *self._model_arguments(),
                self._prompt(),
            ],
            timeout_seconds=self._launch_timeout_seconds,
            environment=self._environment,
        )
        if result.returncode != 0:
            raise RuntimeError(
                f"OpenCode launch exited with {result.returncode}: "
                f"{result.output[-6000:]}"
            )
        trace_ids = [int(value) for value in self._TRACE_PATTERN.findall(result.output)]
        if len(trace_ids) != 1:
            raise RuntimeError(
                f"OpenCode launch reported unexpected trace ids {trace_ids}: "
                f"{result.output[-6000:]}"
            )
        return trace_ids[0], result

    def _model_arguments(self) -> list[str]:
        model = os.environ.get("OPENCODE_E2E_MODEL", "").strip()
        return [] if not model else ["--model", model]

    def _prompt(self) -> str:
        return (
            "You must delegate the repository check by calling the task tool with "
            "the general subagent. The main agent must not use bash, shell, read, "
            "or git itself. Include this exact token in the delegated prompt: "
            f"{self.task_marker}. Tell the subagent to execute exactly "
            "`git rev-parse HEAD` in the current fixture repository and return the "
            "complete commit id. Wait for the subagent result. Then reply with "
            f"`{self.answer_marker} <commit-id>`."
        )
