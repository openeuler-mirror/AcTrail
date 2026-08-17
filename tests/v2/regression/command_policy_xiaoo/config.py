from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.core.loopback_port import resolve_test_port


@dataclass(frozen=True)
class CommandPolicyXiaooConfig(CommonTestConfig):
    operator_config: Path
    operator_config_patch: Path
    xiaoo_binary: Path
    bash_executable: Path
    web_host: str
    web_port: int
    ready_timeout_seconds: float
    evidence_timeout_seconds: float

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "CommandPolicyXiaooConfig":
        common = CommonTestConfig.from_environment(inputs, "COMMAND_POLICY_XIAOO")
        xiaoo = os.environ.get("COMMAND_POLICY_XIAOO_E2E_BINARY") or shutil.which(
            "xiaoo"
        )
        if xiaoo is None:
            raise RuntimeError("real Xiaoo executable is not available")
        bash = Path(
            os.environ.get("COMMAND_POLICY_XIAOO_E2E_BASH", "/usr/bin/bash")
        ).resolve()
        ready_timeout = float(
            os.environ.get("COMMAND_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS", "15")
        )
        evidence_timeout = float(
            os.environ.get("COMMAND_POLICY_XIAOO_E2E_EVIDENCE_TIMEOUT_SECONDS", "15")
        )
        if ready_timeout <= 0 or evidence_timeout <= 0:
            raise RuntimeError("command-policy Xiaoo timeouts must be positive")
        if not bash.is_file() or not os.access(bash, os.X_OK):
            raise RuntimeError(f"Bash executable is unavailable: {bash}")
        return cls(
            **common.as_kwargs(),
            operator_config=inputs.work_dir / "actraild.conf",
            operator_config_patch=inputs.work_dir / "actraild.patch.toml",
            xiaoo_binary=Path(xiaoo).resolve(),
            bash_executable=bash,
            web_host=os.environ.get(
                "COMMAND_POLICY_XIAOO_E2E_WEB_HOST", "127.0.0.1"
            ),
            web_port=resolve_test_port(
                "COMMAND_POLICY_XIAOO_E2E_WEB_PORT",
                attempts=common.drain_attempts,
                connect_timeout_seconds=common.drain_interval_seconds,
            ),
            ready_timeout_seconds=ready_timeout,
            evidence_timeout_seconds=evidence_timeout,
        )
