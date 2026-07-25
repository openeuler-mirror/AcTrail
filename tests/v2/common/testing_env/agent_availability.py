from __future__ import annotations

import os
import shutil
import subprocess


class AgentAvailability:
    def __init__(self):
        self._agent2availability: dict[str, bool] = {}
        self._timeout_seconds = int(
            os.environ.get("ACTRAIL_TEST_AGENT_AVAILABILITY_TIMEOUT_SECONDS", "60")
        )

    def check_agent_availability(self, agent_name: str) -> bool:
        normalized = agent_name.strip().lower()
        if normalized not in self._agent2availability:
            self._agent2availability[normalized] = self._update_agent_availability(
                normalized
            )
        return self._agent2availability[normalized]

    def _update_agent_availability(self, agent_name: str) -> bool:
        checks = {
            "codex": self._check_codex_availability,
            "claude": self._check_claude_availability,
            "xiaoo": self._check_xiaoo_availability,
        }
        check = checks.get(agent_name)
        if check is None:
            raise ValueError(f"unknown agent availability target: {agent_name}")
        return check()

    def _prompt(self) -> str:
        return os.environ.get(
            "ACTRAIL_TEST_AGENT_AVAILABILITY_PROMPT",
            "directly answer hello",
        )

    def _check_codex_availability(self) -> bool:
        return self._run(
            [
                "codex",
                "exec",
                "--ephemeral",
                "-m",
                os.environ.get("ACTRAIL_TEST_CODEX_MODEL", "gpt-5.5"),
                "-c",
                "model_reasoning_effort="
                + os.environ.get("ACTRAIL_TEST_CODEX_REASONING_EFFORT", "low"),
                self._prompt(),
            ]
        )

    def _check_claude_availability(self) -> bool:
        return self._run(["claude", "-p", self._prompt()])

    def _check_xiaoo_availability(self) -> bool:
        return self._run(["xiaoo", "--cli", "run", "-p", self._prompt()])

    def _run(self, command: list[str]) -> bool:
        if shutil.which(command[0]) is None:
            return False
        try:
            result = subprocess.run(
                command,
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return False
        return result.returncode == 0
