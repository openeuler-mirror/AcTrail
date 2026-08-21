from __future__ import annotations

import os
import shutil
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs


@dataclass(frozen=True)
class NetworkPolicyXiaooConfig(CommonTestConfig):
    operator_config: Path
    operator_config_patch: Path
    plugin_package: Path
    xiaoo_binary: Path
    web_host: str
    web_port: int
    ready_timeout_seconds: float
    evidence_timeout_seconds: float

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "NetworkPolicyXiaooConfig":
        common = CommonTestConfig.from_environment(inputs, "NETWORK_POLICY_XIAOO")
        xiaoo = os.environ.get("NETWORK_POLICY_XIAOO_E2E_BINARY") or shutil.which(
            "xiaoo"
        )
        if xiaoo is None:
            raise RuntimeError("real Xiaoo executable is not available")
        plugin_root = Path(
            os.environ.get("ACTRAIL_PLUGIN_DIR", "~/.actrail/plugins")
        ).expanduser()
        if not plugin_root.is_absolute():
            raise RuntimeError("ACTRAIL_PLUGIN_DIR must be an absolute path")
        ready_timeout = float(
            os.environ.get("NETWORK_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS", "15")
        )
        evidence_timeout = float(
            os.environ.get("NETWORK_POLICY_XIAOO_E2E_EVIDENCE_TIMEOUT_SECONDS", "15")
        )
        if ready_timeout <= 0 or evidence_timeout <= 0:
            raise RuntimeError("network-policy Xiaoo timeouts must be positive")
        return cls(
            **common.as_kwargs(),
            operator_config=inputs.work_dir / "actraild.conf",
            operator_config_patch=inputs.work_dir / "actraild.patch.toml",
            plugin_package=(plugin_root / "network-policy-dynamic").resolve(),
            xiaoo_binary=Path(xiaoo).resolve(),
            web_host=os.environ.get(
                "NETWORK_POLICY_XIAOO_E2E_WEB_HOST", "127.0.0.1"
            ),
            web_port=int(os.environ.get("NETWORK_POLICY_XIAOO_E2E_WEB_PORT", "0")),
            ready_timeout_seconds=ready_timeout,
            evidence_timeout_seconds=evidence_timeout,
        )
