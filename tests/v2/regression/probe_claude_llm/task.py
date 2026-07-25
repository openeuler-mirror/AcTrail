from __future__ import annotations

import os
import pwd
import secrets
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult

from .config import ProbeClaudeLLMConfig


class ProbeClaudeLLMTask:
    def __init__(self, config: ProbeClaudeLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self.marker = f"A{secrets.token_hex(5)}"
        self._claude = self._resolve_claude()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self._environment(),
        )

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._claude,
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
            "--print",
            "--output-format",
            "text",
            "--model",
            self._config.model,
            "--no-session-persistence",
            "--safe-mode",
            "--permission-mode",
            "dontAsk",
            "--tools",
            "",
        ]

    def _resolve_claude(self) -> Path:
        if self._config.claude_binary is not None:
            return self._config.claude_binary
        discovered = shutil.which("claude")
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
            candidate = home / ".local/bin/claude"
            if candidate.is_file():
                return candidate
        raise RuntimeError(
            "Claude executable not found; set CLAUDE_E2E_BINARY to its path"
        )

    def _environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        for parent in self._claude.resolve().parents:
            if parent.name == ".local":
                environment["HOME"] = str(parent.parent)
                break
        return environment
