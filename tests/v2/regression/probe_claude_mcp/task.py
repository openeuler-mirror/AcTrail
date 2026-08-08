from __future__ import annotations

import secrets
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import AgentBinaryNotFoundError, TestOutput
from tests.v2.common.mcp_test_support import McpProbeSpec, McpProbeWorkspace
from tests.v2.common.testing_env import AgentBinaryDiscovery

from .config import ProbeClaudeMcpConfig
from .streaming_launch import ClaudeMcpStreamingLaunch


class ProbeClaudeMcpTask:
    def __init__(
        self,
        config: ProbeClaudeMcpConfig,
        runtime: ActrailRuntime,
    ) -> None:
        self._config = config
        self._runtime = runtime
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._claude = self._resolve_claude()
        self._workspace = McpProbeWorkspace(
            config.repo,
            config.artifact_root,
            "probe_claude_mcp",
        )
        try:
            token = secrets.token_hex(6)
            self.final_marker = f"CLAUDE_MCP_FINAL_{token}"
            self.local = self._workspace.spec(
                server_name=config.local_server_name,
                tool_name=config.tool_name,
                marker=f"CLAUDE_STDIO_{token}",
                tool_description_padding_bytes=(
                    config.tool_description_padding_bytes
                ),
            )
        except Exception:
            self._workspace.close()
            raise

    @property
    def binary(self) -> Path:
        return self._claude

    @property
    def expected_calls(self) -> tuple[McpProbeSpec, ...]:
        return (self.local,)

    def run(self, output: TestOutput) -> CommandResult:
        config_path = self._workspace.write_claude_config(self.local)
        launch = ClaudeMcpStreamingLaunch(
            repo=self._config.repo,
            command=self._command(config_path),
            environment=self.environment(),
            workspace=self._workspace,
            expected_calls=self.expected_calls,
            ready_timeout_seconds=self._config.mcp_ready_timeout_seconds,
            ready_poll_interval_seconds=(
                self._config.mcp_ready_poll_interval_seconds
            ),
            launch_timeout_seconds=self._config.launch_timeout_seconds,
            shutdown_timeout_seconds=self._config.command_timeout_seconds,
            output=output,
        )
        return launch.run(self._prompt())

    def require_agent_evidence(self, launch: CommandResult) -> str:
        execution_evidence = self._workspace.require_execution(self.local)
        if self.final_marker not in launch.stdout:
            raise AssertionError(
                "Claude stdout answer does not contain final marker "
                f"{self.final_marker}"
            )
        return execution_evidence

    def environment(self) -> dict[str, str]:
        return self._discovery.environment(self._claude)

    def close(self) -> None:
        self._workspace.close()

    def _command(self, config_path: Path) -> list[Path | str]:
        return [
            *self._runtime.control_command("launch"),
            "--",
            self._claude,
            "--print",
            "--verbose",
            "--output-format",
            "stream-json",
            "--input-format",
            "stream-json",
            "--model",
            self._config.model,
            "--no-session-persistence",
            "--strict-mcp-config",
            "--mcp-config",
            config_path,
            "--permission-mode",
            "dontAsk",
            "--allowedTools",
            self.local.tool_id,
        ]

    def _prompt(self) -> str:
        return (
            f"Use {self.local.tool_id} with "
            f'{{"marker":"{self.local.marker}"}}. '
            f'After the result returns, reply with "{self.final_marker}".'
        )

    def _resolve_claude(self) -> Path:
        configured = self._config.claude_binary
        if configured is not None:
            if AgentBinaryDiscovery.is_executable(configured):
                return configured
            raise AgentBinaryNotFoundError(
                "configured Claude executable is unavailable: "
                f"{configured}"
            )
        binary = self._discovery.resolve("CLAUDE_E2E_BINARY", "claude")
        if binary is None:
            raise AgentBinaryNotFoundError(
                "Claude executable not found; set CLAUDE_E2E_BINARY to its path"
            )
        return binary
