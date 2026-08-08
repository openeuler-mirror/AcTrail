"""Uniform agent binary discovery shared by all V2 regression cases."""

from __future__ import annotations

import os
import pwd
import shutil
from pathlib import Path


class AgentBinaryDiscovery:
    """Resolve any agent binary through one shared search chain.

    Every supported agent (xiaoo, claude, codex, pi, opencode) follows the
    same order:

    1. the per-agent environment variable (e.g. XIAOO_E2E_BINARY);
    2. a PATH lookup via shutil.which;
    3. home-local bin directories under the current user, the checkout
       owner, and the sudo invoker (regressions run as root via sudo, so
       binaries installed under a developer home are otherwise invisible).

    environment() keeps HOME aligned to the user owning a home-local binary
    so the agent starts with the right configuration.
    """

    _HOME_BIN_DIRS = (".cargo/bin", ".local/bin")

    def __init__(self, repo: Path) -> None:
        self._repo = repo.resolve()

    def resolve(self, env_var: str, executable: str) -> Path | None:
        configured = os.environ.get(env_var)
        if configured:
            candidate = Path(configured)
            if self.is_executable(candidate):
                return candidate
            return None
        discovered = shutil.which(executable)
        if discovered:
            return Path(discovered)
        return self._find_in_homes(executable)

    def environment(self, binary: Path) -> dict[str, str]:
        environment = os.environ.copy()
        home = self._home_of(binary)
        if home is not None:
            environment["HOME"] = str(home)
        return environment

    @staticmethod
    def is_executable(path: Path) -> bool:
        return path.is_file() and os.access(path, os.X_OK)

    def _find_in_homes(self, executable: str) -> Path | None:
        for home in self._candidate_homes():
            for bin_dir in self._HOME_BIN_DIRS:
                candidate = home / bin_dir / executable
                if self.is_executable(candidate):
                    return candidate
        return None

    def _home_of(self, binary: Path) -> Path | None:
        resolved = binary.resolve()
        for home in self._candidate_homes():
            for bin_dir in self._HOME_BIN_DIRS:
                try:
                    resolved.relative_to(home / bin_dir)
                except ValueError:
                    continue
                return home
        return None

    def _candidate_homes(self) -> set[Path]:
        homes: set[Path] = set()
        try:
            homes.add(Path(pwd.getpwuid(os.getuid()).pw_dir))
        except KeyError:
            pass
        try:
            homes.add(Path(pwd.getpwuid(self._repo.stat().st_uid).pw_dir))
        except KeyError:
            pass
        invoking_user = os.environ.get("SUDO_USER")
        if invoking_user and invoking_user != "root":
            try:
                homes.add(Path(pwd.getpwnam(invoking_user).pw_dir))
            except KeyError:
                pass
        return homes
