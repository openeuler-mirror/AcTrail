from __future__ import annotations

import json
import os
import pwd
import secrets
import shutil
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.errors import AgentBinaryNotFoundError
from tests.v2.common.mcp_test_support import McpProbeSpec, McpProbeWorkspace

from .config import ProbeCodexMcpConfig


class ProbeCodexMcpTask:
    def __init__(
        self,
        config: ProbeCodexMcpConfig,
        runtime: ActrailRuntime,
    ) -> None:
        self._config = config
        self._runtime = runtime
        self._codex = self._resolve_codex()
        self._workspace = McpProbeWorkspace(
            config.repo,
            config.artifact_root,
            "probe_codex_mcp",
        )
        try:
            token = secrets.token_hex(6)
            self.final_marker = f"CODEX_MCP_FINAL_{token}"
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
    def expected_calls(self) -> tuple[McpProbeSpec, ...]:
        return (self.local,)

    def run(self) -> CommandResult:
        return self._runtime.run(
            self._command(),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=self.environment(),
        )

    def require_agent_evidence(self, launch: CommandResult) -> str:
        if self.final_marker not in launch.stdout:
            raise AssertionError(
                "Codex stdout answer does not contain final marker "
                f"{self.final_marker}"
            )
        return self._workspace.require_execution(self.local)

    def environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        for parent in self._codex.resolve().parents:
            if parent.name == ".codex":
                environment["HOME"] = str(parent.parent)
                environment["CODEX_HOME"] = str(parent)
                break
        return environment

    def close(self) -> None:
        self._workspace.close()

    def _command(self) -> list[Path | str]:
        command, arguments = self._workspace.stdio_command(self.local)
        server_key = f"mcp_servers.{self.local.server_name}"
        overrides = [
            f"model_reasoning_effort={self._config.reasoning_effort}",
            f"{server_key}.command={json.dumps(command)}",
            f"{server_key}.args={json.dumps(arguments)}",
        ]
        argv: list[Path | str] = [
            *self._runtime.control_command("launch"),
            "--",
            self._codex,
            "exec",
            "--ephemeral",
            "-m",
            self._config.model,
        ]
        for override in overrides:
            argv.extend(["-c", override])
        argv.append(self._prompt())
        return argv

    def _prompt(self) -> str:
        return (
            f"Use {self.local.tool_id} with "
            f'{{"marker":"{self.local.marker}"}}. '
            f'After the result returns, reply with "{self.final_marker}".'
        )

    def _resolve_codex(self) -> Path:
        if self._config.codex_binary is not None:
            if self._is_executable(self._config.codex_binary):
                return self._config.codex_binary
            raise AgentBinaryNotFoundError(
                "configured Codex executable is unavailable: "
                f"{self._config.codex_binary}"
            )
        discovered = shutil.which("codex")
        if discovered:
            return Path(discovered)
        homes = {
            Path(pwd.getpwuid(os.getuid()).pw_dir),
            Path(pwd.getpwuid(self._config.repo.stat().st_uid).pw_dir),
        }
        invoking_user = os.environ.get("SUDO_USER")
        if invoking_user and invoking_user != "root":
            homes.add(Path(pwd.getpwnam(invoking_user).pw_dir))
        for home in homes:
            candidate = home / ".local/bin/codex"
            if self._is_executable(candidate):
                return candidate
        raise AgentBinaryNotFoundError(
            "Codex executable not found; set CODEX_E2E_BINARY to its path"
        )

    @staticmethod
    def _is_executable(path: Path) -> bool:
        return path.is_file() and os.access(path, os.X_OK)
