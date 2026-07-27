from __future__ import annotations

import os
import pwd
import secrets
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.errors import AgentBinaryNotFoundError

from .config import ProbeCodexLLMConfig


class ProbeCodexLLMTask:
    def __init__(self, config: ProbeCodexLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self.marker = f"A{secrets.token_hex(5)}"
        self._codex = self._resolve_codex()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._codex

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._codex,
            "exec",
            "--ephemeral",
            "-m",
            self._config.model,
            "-c",
            f"model_reasoning_effort={self._config.reasoning_effort}",
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
        ]

    def _resolve_codex(self) -> Path:
        if self._config.codex_binary is not None:
            if self._is_executable(self._config.codex_binary):
                return self._config.codex_binary
            raise AgentBinaryNotFoundError(
                f"configured Codex executable is unavailable: "
                f"{self._config.codex_binary}"
            )
        discovered = shutil.which("codex")
        if discovered:
            return Path(discovered)
        homes = {
            Path(pwd.getpwuid(os.getuid()).pw_dir),
            Path(pwd.getpwuid(self._config.repo.stat().st_uid).pw_dir),
        }
        invoking_user = os.environ.get("SUDO_USER")
        if invoking_user and invoking_user != "root":
            homes.add(Path(pwd.getpwnam(invoking_user).pw_dir))
        for home in homes:
            candidate = home / ".local/bin/codex"
            if self._is_executable(candidate):
                return candidate
        raise AgentBinaryNotFoundError(
            "Codex executable not found; set CODEX_E2E_BINARY to its path"
        )

    @staticmethod
    def _is_executable(path: Path) -> bool:
        return path.is_file() and os.access(path, os.X_OK)

    def environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        for parent in self._codex.resolve().parents:
            if parent.name == ".codex":
                environment["HOME"] = str(parent.parent)
                environment["CODEX_HOME"] = str(parent)
                break
        return environment
