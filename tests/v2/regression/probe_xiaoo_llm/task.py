from __future__ import annotations

import os
import pwd
import secrets
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.errors import AgentBinaryNotFoundError

from .config import ProbeXiaooLLMConfig


class ProbeXiaooLLMTask:
    def __init__(self, config: ProbeXiaooLLMConfig, runtime: ActrailRuntime):
        self._config = config
        self._runtime = runtime
        self.marker = f"A{secrets.token_hex(5)}"
        self._xiaoo = self._resolve_xiaoo()

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    @property
    def binary(self) -> Path:
        return self._xiaoo

    def _command(self) -> list[Path | str]:
        return [
            self._runtime.actrailctl,
            "launch",
            "--",
            self._xiaoo,
            "--cli",
            "run",
            "--no-tools",
            "--max-turns",
            "1",
            "--prompt",
            f'Reply with exactly "{self.marker}" and nothing else. Do not use tools.',
        ]

    def _resolve_xiaoo(self) -> Path:
        if self._config.xiaoo_binary is not None:
            if self._is_executable(self._config.xiaoo_binary):
                return self._config.xiaoo_binary
            raise AgentBinaryNotFoundError(
                f"configured xiaoO executable is unavailable: "
                f"{self._config.xiaoo_binary}"
            )
        discovered = shutil.which("xiaoo")
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
            candidate = home / ".cargo/bin/xiaoo"
            if self._is_executable(candidate):
                return candidate
        raise AgentBinaryNotFoundError(
            "xiaoO executable not found; set XIAOO_E2E_BINARY to its path"
        )

    @staticmethod
    def _is_executable(path: Path) -> bool:
        return path.is_file() and os.access(path, os.X_OK)

    def environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        resolved = self._xiaoo.resolve()
        for parent in resolved.parents:
            if parent.name == ".cargo":
                environment["HOME"] = str(parent.parent)
                break
        return environment
