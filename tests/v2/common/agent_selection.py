from __future__ import annotations

import os
import pwd
import shutil
from dataclasses import dataclass
from pathlib import Path

from .testing_context import TestingContextSingleton


@dataclass(frozen=True)
class AgentSelection:
    kind: str
    binary: Path
    environment: dict[str, str]

    def command(self, prompt: str) -> list[Path | str]:
        if self.kind == "xiaoo":
            return [
                self.binary,
                "--cli",
                "run",
                "--no-tools",
                "--max-turns",
                "1",
                "--prompt",
                prompt,
            ]
        if self.kind == "pi":
            return [self.binary, "-p", prompt, "--no-session"]
        if self.kind == "opencode":
            return [self.binary, "run", prompt]
        if self.kind == "claude":
            return [
                self.binary,
                prompt,
                "--print",
                "--output-format",
                "text",
                "--model",
                os.environ.get("CLAUDE_E2E_MODEL", "sonnet"),
                "--no-session-persistence",
                "--safe-mode",
                "--permission-mode",
                "dontAsk",
                "--tools",
                "",
            ]
        if self.kind == "codex":
            return [
                self.binary,
                "exec",
                "--ephemeral",
                "-m",
                os.environ.get("CODEX_E2E_MODEL", "gpt-5.5"),
                "-c",
                "model_reasoning_effort="
                + os.environ.get("CODEX_E2E_REASONING_EFFORT", "low"),
                prompt,
            ]
        raise ValueError(f"unsupported agent kind: {self.kind}")


class AgentSelector:
    _CANDIDATES: tuple[tuple[str, str, str], ...] = (
        ("xiaoo", "XIAOO_E2E_BINARY", "xiaoo"),
        ("pi", "PI_E2E_BINARY", "pi"),
        ("opencode", "OPENCODE_E2E_BINARY", "opencode"),
        ("claude", "CLAUDE_E2E_BINARY", "claude"),
        ("codex", "CODEX_E2E_BINARY", "codex"),
    )

    def __init__(self, repo: Path):
        self._repo = repo

    def select(
        self,
        test_context: TestingContextSingleton,
    ) -> AgentSelection | None:
        for kind, variable, executable in self._CANDIDATES:
            binary = self._resolve_binary(kind, variable, executable)
            if binary is None:
                continue
            environment = self._environment(kind, binary)
            if test_context.check_agent_availability(
                kind,
                binary,
                environment,
            ):
                return AgentSelection(kind, binary, environment)
        return None

    def _resolve_binary(
        self,
        kind: str,
        variable: str,
        executable: str,
    ) -> Path | None:
        configured = os.environ.get(variable)
        if configured:
            candidate = Path(configured)
            return candidate if self._is_executable(candidate) else None
        discovered = shutil.which(executable)
        if discovered:
            return Path(discovered)
        if kind == "xiaoo":
            return self._resolve_xiaoo_fallback()
        return None

    def _resolve_xiaoo_fallback(self) -> Path | None:
        homes = {
            Path(pwd.getpwuid(os.getuid()).pw_dir),
            Path(pwd.getpwuid(self._repo.stat().st_uid).pw_dir),
        }
        invoking_user = os.environ.get("SUDO_USER")
        if invoking_user and invoking_user != "root":
            homes.add(Path(pwd.getpwnam(invoking_user).pw_dir))
        for home in homes:
            candidate = home / ".cargo/bin/xiaoo"
            if self._is_executable(candidate):
                return candidate
        return None

    @staticmethod
    def _is_executable(path: Path) -> bool:
        return path.is_file() and os.access(path, os.X_OK)

    @staticmethod
    def _environment(kind: str, binary: Path) -> dict[str, str]:
        environment = os.environ.copy()
        if kind != "xiaoo":
            return environment
        resolved = binary.resolve()
        for parent in resolved.parents:
            if parent.name == ".cargo":
                environment["HOME"] = str(parent.parent)
                break
        return environment
