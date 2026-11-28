from __future__ import annotations

import json
import secrets
from pathlib import Path

from tests.e2e.common.agents import AgentFactory
from tests.e2e.common.core import AgentBinaryNotFoundError
from tests.e2e.common.mcp_test_support import McpProbeSpec, McpProbeWorkspace
from tests.e2e.common.testing_env import AgentBinaryDiscovery

from .config import ProbeCodexMcpConfig

from .constants import CASE_CONSTANTS


class ProbeCodexMcpTask:
    def __init__(
        self,
        config: ProbeCodexMcpConfig,
    ) -> None:
        self._config = config
        self._discovery = AgentBinaryDiscovery(config.repo)
        self._codex = self._resolve_codex()
        self._native_codex = self._resolve_native_codex()
        self._workspace = McpProbeWorkspace(
            config.repo,
            config.artifact_root,
            "probe_codex_mcp",
        )
        try:
            token = secrets.token_hex(CASE_CONSTANTS.marker_random_bytes)
            self.local = self._workspace.spec(
                server_name=config.local_server_name,
                tool_name=config.tool_name,
                marker=f"CODEX_STDIO_{token}",
                tool_description_padding_bytes=(
                    config.tool_description_padding_bytes
                ),
            )
        except Exception:
            self._workspace.close()
            raise

    @property
    def binary(self) -> Path:
        return self._codex

    @property
    def native_binary(self) -> Path:
        return self._native_codex

    @property
    def expected_calls(self) -> tuple[McpProbeSpec, ...]:
        return (self.local,)

    def run(self):
        with self.create_agent().launchWithAcTrailCtl(stream_mode=False) as agent:
            agent.run(self._prompt())
            agent.join_all(self._config.launch_timeout_seconds)
            result = agent.results[-1]
            if not result.succeeded:
                raise AssertionError(result.error or "Codex MCP task failed")
            launch = agent.launch_result
        if agent.returncode != 0:
            raise AssertionError(f"Codex launch exited with {agent.returncode}")
        return launch, result

    def __enter__(self):
        return self

    def __exit__(self, exception_type, exception, traceback):
        self.close()

    def require_agent_evidence(self) -> str:
        return self._workspace.require_execution(self.local)

    def tool_execution_count(self) -> int:
        return self._workspace.execution_count(self.local)

    def replied_no_tool(self, output: str) -> bool:
        return "NO_MCP_TOOL" in output

    def environment(self) -> dict[str, str]:
        environment = self._discovery.environment(self._codex)
        for parent in self._codex.resolve().parents:
            if parent.name == ".codex":
                environment["HOME"] = str(parent.parent)
                environment["CODEX_HOME"] = str(parent)
                break
        return environment

    def close(self) -> None:
        self._workspace.close()

    def create_agent(self):
        command, arguments = self._workspace.stdio_command(self.local)
        server_key = f"mcp_servers.{self.local.server_name}"
        overrides = [
            f"model={json.dumps(self._config.model)}",
            f"model_reasoning_effort={self._config.reasoning_effort}",
            f"{server_key}.command={json.dumps(command)}",
            f"{server_key}.args={json.dumps(arguments)}",
        ]
        options = tuple(value for override in overrides for value in ("-c", override))
        return AgentFactory.codex(
            binary=self._codex, arguments=("--skip-git-repo-check", *options),
            environment=self.environment(),
        )

    def _prompt(self) -> str:
        return (
            f"Use the {self.local.tool_id} MCP tool exactly once with "
            f'{{"marker":"{self.local.marker}"}}, then describe the tool result.'
        )

    def _resolve_codex(self) -> Path:
        configured = self._config.codex_binary
        if configured is not None:
            if AgentBinaryDiscovery.is_executable(configured):
                return configured
            raise AgentBinaryNotFoundError(
                "configured Codex executable is unavailable: "
                f"{configured}"
            )
        binary = self._discovery.resolve("CODEX_E2E_BINARY", "codex")
        if binary is None:
            raise AgentBinaryNotFoundError(
                "Codex executable not found; set CODEX_E2E_BINARY to its path"
            )
        return binary

    def _resolve_native_codex(self) -> Path:
        binary = self._discovery.codex_native_binary_for(self._codex)
        if binary is None:
            raise AgentBinaryNotFoundError(
                "native Codex executable not found; set "
                "CODEX_E2E_NATIVE_BINARY to its path"
            )
        return binary
