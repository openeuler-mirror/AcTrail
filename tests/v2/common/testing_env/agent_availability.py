from __future__ import annotations

import os
import shutil
import subprocess
from pathlib import Path
from typing import Mapping

from .agent_discovery import AgentBinaryDiscovery, default_claude_model


class AgentAvailability:
    def __init__(self):
        self._agent2availability: dict[tuple[str, str], bool] = {}
        self._timeout_seconds = int(
            os.environ.get("ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS", "60")
        )

    def check_agent_availability(
        self,
        agent_name: str,
        binary: Path | str | None = None,
        environment: Mapping[str, str] | None = None,
    ) -> bool:
        normalized = agent_name.strip().lower()
        selected_binary = str(binary or normalized)
        cache_key = (normalized, selected_binary)
        if cache_key not in self._agent2availability:
            self._agent2availability[cache_key] = self._update_agent_availability(
                normalized,
                selected_binary,
                environment,
            )
        return self._agent2availability[cache_key]

    def _update_agent_availability(
        self,
        agent_name: str,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        checks = {
            "codex": self._check_codex_availability,
            "claude": self._check_claude_availability,
            "opencode": self._check_opencode_availability,
            "pi": self._check_pi_availability,
            "qodercli": self._check_qodercli_availability,
            "xiaoo": self._check_xiaoo_availability,
        }
        check = checks.get(agent_name)
        if check is None:
            raise ValueError(f"unknown agent availability target: {agent_name}")
        return check(binary, environment)

    def _prompt(self) -> str:
        return os.environ.get(
            "ACTRAIL_TEST_AGENT_AVAILABILITY_PROMPT",
            "directly answer hello",
        )

    def _check_codex_availability(
        self,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        return self._run(
            [
                binary,
                "exec",
                "--ephemeral",
                "-m",
                self._codex_model(binary),
                "-c",
                "model_reasoning_effort="
                + os.environ.get("CODEX_E2E_REASONING_EFFORT", "low"),
                self._prompt(),
            ],
            environment,
        )

    def _codex_model(self, binary: str) -> str:
        configured = os.environ.get("CODEX_E2E_MODEL")
        if configured:
            return configured
        try:
            model = AgentBinaryDiscovery(Path.cwd()).default_codex_model_for_binary(
                Path(binary)
            )
        except Exception:
            model = None
        return model or "gpt-5.5"

    def _check_claude_availability(
        self,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        return self._run(
            [
                binary,
                "-p",
                self._prompt(),
                "--model",
                default_claude_model(),
                "--no-session-persistence",
                "--safe-mode",
                "--permission-mode",
                "dontAsk",
                "--tools",
                "",
            ],
            environment,
        )

    def _check_pi_availability(
        self,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        return self._run(
            [binary, "-p", self._prompt(), "--no-session"],
            environment,
        )

    def _check_opencode_availability(
        self,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        return self._run(
            [binary, "run", self._prompt()],
            environment,
        )

    def _check_qodercli_availability(
        self,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        return self._run(
            [
                binary,
                "--no-session-persistence",
                "-p",
                self._prompt(),
                "--tools",
                "",
            ],
            environment,
        )

    def _check_xiaoo_availability(
        self,
        binary: str,
        environment: Mapping[str, str] | None,
    ) -> bool:
        return self._run(
            [
                binary,
                "--cli",
                "run",
                "--no-tools",
                "--max-turns",
                "1",
                "--prompt",
                self._prompt(),
            ],
            environment,
        )

    def _run(
        self,
        command: list[str],
        environment: Mapping[str, str] | None,
    ) -> bool:
        path = None if environment is None else environment.get("PATH")
        if shutil.which(command[0], path=path) is None:
            return False
        try:
            result = subprocess.run(
                command,
                env=None if environment is None else dict(environment),
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
        return result.returncode == 0
