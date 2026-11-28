"""Resolve scenario-owned configuration from run directories and environment."""
from dataclasses import dataclass
import os
from pathlib import Path

from tests.e2e.common.core import TestRunDirectories
from .constants import CASE_CONSTANTS


@dataclass(frozen=True)
class NetworkRequestCoverageConfig:
    repo: Path
    bin_dir: Path
    work_dir: Path
    timeout_seconds: int
    max_turns: int
    error_model: str
    prompt: str
    xiaoo_binary: Path | None

    @classmethod
    def from_environment(cls, directories: TestRunDirectories):
        prefix = "NETWORK_REQUEST_COVERAGE_E2E_"
        binary = os.environ.get(prefix + "XIAOO_BINARY", os.environ.get("XIAOO_E2E_BINARY"))
        config = cls(
            directories.repo, directories.bin_dir.resolve(), directories.work_dir,
            int(os.environ.get(prefix + "TIMEOUT_SECONDS", CASE_CONSTANTS.timeout_seconds)),
            int(os.environ.get(prefix + "MAX_TURNS", CASE_CONSTANTS.max_turns)),
            os.environ.get(prefix + "ERROR_MODEL", CASE_CONSTANTS.error_model),
            os.environ.get(prefix + "PROMPT", CASE_CONSTANTS.prompt),
            Path(binary) if binary else None,
        )
        if config.timeout_seconds <= 0 or config.max_turns <= 0:
            raise ValueError("Network request coverage timeouts and turn limit must be positive")
        if not config.error_model.strip() or not config.prompt.strip():
            raise ValueError("Network request coverage model and prompt must be nonempty")
        return config
