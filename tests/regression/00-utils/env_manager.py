"""Shared regression environment state."""

from __future__ import annotations

import os
import platform
import shutil
import subprocess
import sys
from pathlib import Path

from model import CommandResult


# Conventional exit code used by shell timeout wrappers.
TIMEOUT_RETURN_CODE = 124
DEFAULT_OPERATOR_CONFIG_PATH = Path("/etc/actrail/actraild.conf")


class EnvManager:
    """Cache environment facts shared by regression cases."""

    _instance: "EnvManager | None" = None

    def __init__(
        self,
        regression_root: Path,
        bin_dir: str,
        output_tail_chars: int,
        output_dir: Path,
    ) -> None:
        self.regression_root = regression_root
        self.repo_root = regression_root.parents[1]
        self.bin_dir = self.resolve_repo_path(bin_dir)
        self.python = sys.executable
        self.output_tail_chars = output_tail_chars
        self.output_dir = output_dir
        self._which: dict[str, str | None] = {}
        self._executable_candidates: dict[str, list[Path]] = {}
        self._release_binary: dict[str, Path | None] = {}

    @classmethod
    def configure(
        cls,
        regression_root: Path,
        bin_dir: str,
        output_tail_chars: int,
        output_dir: Path,
    ) -> "EnvManager":
        cls._instance = EnvManager(regression_root, bin_dir, output_tail_chars, output_dir)
        return cls._instance

    @classmethod
    def instance(cls) -> "EnvManager":
        if cls._instance is None:
            raise RuntimeError("EnvManager is not configured")
        return cls._instance

    def resolve_repo_path(self, raw: str) -> Path:
        path = Path(raw)
        return path if path.is_absolute() else self.repo_root / path

    def which(self, name: str) -> str | None:
        if name not in self._which:
            self._which[name] = shutil.which(name)
        return self._which[name]

    def executable_candidates(self, name: str) -> list[Path]:
        if name not in self._executable_candidates:
            candidates: list[Path] = []
            seen: set[Path] = set()
            for raw_dir in os.environ.get("PATH", "").split(os.pathsep):
                if not raw_dir:
                    continue
                path = Path(raw_dir) / name
                if not self.is_executable(path):
                    continue
                key = path.absolute()
                if key in seen:
                    continue
                seen.add(key)
                candidates.append(path)
            self._executable_candidates[name] = candidates
        return list(self._executable_candidates[name])

    def resolve_executable_reference(self, raw: str) -> Path | None:
        path = Path(raw)
        if path.parent != Path("."):
            return path if self.is_executable(path) else None
        resolved = self.which(raw)
        return Path(resolved) if resolved else None

    def is_executable(self, path: Path) -> bool:
        return path.exists() and path.is_file() and os.access(path, os.X_OK)

    def python_candidates(self) -> list[Path]:
        candidates: list[Path] = []
        add_unique(candidates, Path(self.python))
        virtual_env = os.environ.get("VIRTUAL_ENV")
        if virtual_env:
            add_unique(candidates, Path(virtual_env) / "bin" / "python")
        add_unique(candidates, self.repo_root / ".venv" / "bin" / "python")
        for name in ("python3", "python"):
            for path in self.executable_candidates(name):
                add_unique(candidates, path)
        return [path for path in candidates if self.is_executable(path)]

    def release_binary(self, name: str) -> Path | None:
        if name not in self._release_binary:
            path = self.bin_dir / name
            self._release_binary[name] = path if path.exists() else None
        return self._release_binary[name]

    def release_binaries_ready(self) -> bool:
        return all(self.release_binary(name) for name in ("actraild", "actrailctl", "actrailviewer"))

    def default_operator_config_path(self) -> Path:
        return DEFAULT_OPERATOR_CONFIG_PATH

    def is_root(self) -> bool:
        return os.geteuid() == 0

    def platform_summary(self) -> str:
        return f"{platform.platform()} arch={platform.machine()} uid={os.geteuid()}"

    def has_env(self, name: str) -> bool:
        return bool(os.environ.get(name))

    def proxy_summary(self) -> str:
        names = (
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "NO_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
            "no_proxy",
            "ftp_proxy",
        )
        present = [name for name in names if os.environ.get(name)]
        return ", ".join(present) if present else "no proxy variables set"

    def langgraph_python(self) -> str:
        return os.environ.get("LANGGRAPH_PYTHON", self.python)

    def run(
        self,
        command: list[str],
        *,
        cwd: Path | None = None,
        env: dict[str, str] | None = None,
        timeout: float | None = None,
    ) -> CommandResult:
        process_env = os.environ.copy()
        if env:
            process_env.update(env)
        try:
            result = subprocess.run(
                command,
                cwd=cwd or self.repo_root,
                env=process_env,
                text=True,
                capture_output=True,
                check=False,
                timeout=timeout,
            )
            return CommandResult(command, result.returncode, result.stdout, result.stderr)
        except subprocess.TimeoutExpired as error:
            stdout = decode_timeout_output(error.stdout)
            stderr = decode_timeout_output(error.stderr)
            return CommandResult(command, TIMEOUT_RETURN_CODE, stdout, stderr)

    def output_tail(self, text: str) -> str:
        if self.output_tail_chars <= 0:
            return text
        return text[-self.output_tail_chars :] if len(text) > self.output_tail_chars else text


def add_unique(items: list[Path], path: Path) -> None:
    if path not in items:
        items.append(path)


def decode_timeout_output(value: str | bytes | None) -> str:
    if value is None:
        return ""
    if isinstance(value, bytes):
        return value.decode(errors="replace")
    return value
