from __future__ import annotations

import os
from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestCaseInputs
from tests.v2.common.mcp_test_support import STDIO_CAPTURE_ABI_MAX_BYTES


@dataclass(frozen=True)
class ProbeCodexMcpConfig(CommonTestConfig):
    model: str
    reasoning_effort: str
    codex_binary: Path | None
    artifact_root: Path
    local_server_name: str
    tool_name: str
    tool_description_padding_bytes: int

    def __post_init__(self) -> None:
        if not self.model.strip():
            raise ValueError("Codex MCP model must be nonempty")
        if not self.reasoning_effort.strip():
            raise ValueError("Codex MCP reasoning effort must be nonempty")
        if self.command_timeout_seconds <= 0:
            raise ValueError("Codex MCP command timeout must be positive")
        if self.launch_timeout_seconds <= 0:
            raise ValueError("Codex MCP launch timeout must be positive")
        if (
            self.tool_description_padding_bytes
            <= STDIO_CAPTURE_ABI_MAX_BYTES
        ):
            raise ValueError(
                "Codex MCP tool description padding must exceed the 4095-byte "
                "stdio capture ABI boundary"
            )

    @classmethod
    def from_environment(
        cls,
        inputs: TestCaseInputs,
    ) -> "ProbeCodexMcpConfig":
        common = CommonTestConfig.from_environment(inputs, "CODEX_MCP")
        configured_binary = os.environ.get("CODEX_E2E_BINARY")
        return cls(
            **common.as_kwargs(),
            model=os.environ.get("CODEX_E2E_MODEL", "gpt-5.5"),
            reasoning_effort=os.environ.get(
                "CODEX_E2E_REASONING_EFFORT",
                "low",
            ),
            codex_binary=Path(configured_binary) if configured_binary else None,
            artifact_root=Path(
                os.environ.get(
                    "CODEX_MCP_E2E_ARTIFACT_ROOT",
                    "temp/v2-regression/mcp",
                )
            ),
            local_server_name=os.environ.get(
                "CODEX_MCP_E2E_LOCAL_SERVER_NAME",
                "actrail_codex_stdio",
            ),
            tool_name=os.environ.get(
                "CODEX_MCP_E2E_TOOL_NAME",
                "emit_marker",
            ),
            tool_description_padding_bytes=int(
                os.environ.get(
                    "CODEX_MCP_E2E_TOOL_DESCRIPTION_PADDING_BYTES",
                    "8192",
                )
            ),
        )
