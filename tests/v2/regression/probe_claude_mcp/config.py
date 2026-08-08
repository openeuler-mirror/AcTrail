from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.mcp_test_support import STDIO_CAPTURE_ABI_MAX_BYTES
from tests.v2.common.testing_env import default_claude_model


@dataclass(frozen=True)
class ProbeClaudeMcpConfig(CommonTestConfig):
    model: str
    claude_binary: Path | None
    artifact_root: Path
    local_server_name: str
    tool_name: str
    mcp_ready_timeout_seconds: float
    mcp_ready_poll_interval_seconds: float
    tool_description_padding_bytes: int

    def __post_init__(self) -> None:
        if not self.model.strip():
            raise ValueError("Claude MCP model must be nonempty")
        if self.command_timeout_seconds <= 0:
            raise ValueError("Claude MCP command timeout must be positive")
        if self.launch_timeout_seconds <= 0:
            raise ValueError("Claude MCP launch timeout must be positive")
        if self.mcp_ready_timeout_seconds <= 0:
            raise ValueError("Claude MCP ready timeout must be positive")
        if self.mcp_ready_poll_interval_seconds <= 0:
            raise ValueError("Claude MCP ready poll interval must be positive")
        if self.mcp_ready_timeout_seconds >= self.launch_timeout_seconds:
            raise ValueError(
                "Claude MCP ready timeout must be shorter than launch timeout"
            )
        if (
            self.tool_description_padding_bytes
            <= STDIO_CAPTURE_ABI_MAX_BYTES
        ):
            raise ValueError(
                "Claude MCP tool description padding must exceed the 4095-byte "
                "stdio capture ABI boundary"
            )

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProbeClaudeMcpConfig":
        common = CommonTestConfig.from_environment(inputs, "CLAUDE_MCP")
        configured_binary = os.environ.get("CLAUDE_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            model=default_claude_model(),
            claude_binary=Path(configured_binary) if configured_binary else None,
            artifact_root=Path(
                os.environ.get(
                    "CLAUDE_MCP_E2E_ARTIFACT_ROOT",
                    "temp/v2-regression/mcp",
                )
            ),
            local_server_name=os.environ.get(
                "CLAUDE_MCP_E2E_LOCAL_SERVER_NAME",
                "actrail_claude_stdio",
            ),
            tool_name=os.environ.get(
                "CLAUDE_MCP_E2E_TOOL_NAME",
                "emit_marker",
            ),
            mcp_ready_timeout_seconds=float(
                os.environ.get(
                    "CLAUDE_MCP_E2E_READY_TIMEOUT_SECONDS",
                    "60",
                )
            ),
            mcp_ready_poll_interval_seconds=float(
                os.environ.get(
                    "CLAUDE_MCP_E2E_READY_POLL_INTERVAL_SECONDS",
                    "0.05",
                )
            ),
            tool_description_padding_bytes=int(
                os.environ.get(
                    "CLAUDE_MCP_E2E_TOOL_DESCRIPTION_PADDING_BYTES",
                    "8192",
                )
            ),
        )
