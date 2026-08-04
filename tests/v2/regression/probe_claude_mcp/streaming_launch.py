from __future__ import annotations

import json
import os
import signal
import subprocess
import time
from pathlib import Path

from tests.v2.common.actrail_runtime import CommandResult
from tests.v2.common.mcp_test_support import McpProbeSpec, McpProbeWorkspace
from tests.v2.common.output import TestOutput


class ClaudeMcpStreamingLaunch:
    def __init__(
        self,
        *,
        repo: Path,
        command: list[Path | str],
        environment: dict[str, str],
        workspace: McpProbeWorkspace,
        expected_calls: tuple[McpProbeSpec, ...],
        ready_timeout_seconds: float,
        ready_poll_interval_seconds: float,
        launch_timeout_seconds: float,
        shutdown_timeout_seconds: float,
        output: TestOutput,
    ) -> None:
        if not expected_calls:
            raise ValueError("Claude MCP streaming launch requires a probe")
        for label, value in (
            ("ready timeout", ready_timeout_seconds),
            ("ready poll interval", ready_poll_interval_seconds),
            ("launch timeout", launch_timeout_seconds),
            ("shutdown timeout", shutdown_timeout_seconds),
        ):
            if value <= 0:
                raise ValueError(f"Claude MCP {label} must be positive")
        if ready_timeout_seconds >= launch_timeout_seconds:
            raise ValueError(
                "Claude MCP ready timeout must be shorter than launch timeout"
            )
        self._repo = repo
        self._command = tuple(str(argument) for argument in command)
        self._environment = environment
        self._workspace = workspace
        self._expected_calls = expected_calls
        self._ready_timeout_seconds = ready_timeout_seconds
        self._ready_poll_interval_seconds = ready_poll_interval_seconds
        self._launch_timeout_seconds = launch_timeout_seconds
        self._shutdown_timeout_seconds = shutdown_timeout_seconds
        self._output = output

    def run(self, prompt: str) -> CommandResult:
        process = subprocess.Popen(
            self._command,
            cwd=self._repo,
            env=self._environment,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
            start_new_session=True,
        )
        started_at = time.monotonic()
        try:
            self._wait_for_tool_listings(process)
            self._send_prompt(process, prompt)
            remaining = self._launch_timeout_seconds - (
                time.monotonic() - started_at
            )
            if remaining <= 0:
                raise TimeoutError(
                    "Claude MCP launch exhausted its timeout before the agent turn"
                )
            stdout, stderr = process.communicate(timeout=remaining)
        except Exception as error:
            stdout, stderr, forced = self._terminate_owned_group(process)
            self._output.command_output(stdout, stderr)
            suffix = " and required SIGKILL" if forced else ""
            raise RuntimeError(
                f"Claude MCP streaming launch failed{suffix}: {error}; "
                f"stdout_tail={stdout[-4000:]!r} "
                f"stderr_tail={stderr[-4000:]!r}"
            ) from error
        self._output.command_output(stdout, stderr)
        return CommandResult(
            argv=self._command,
            returncode=process.returncode,
            stdout=stdout,
            stderr=stderr,
        )

    def _wait_for_tool_listings(
        self,
        process: subprocess.Popen[str],
    ) -> None:
        deadline = time.monotonic() + self._ready_timeout_seconds
        while True:
            pending = [
                probe.tool_id
                for probe in self._expected_calls
                if not self._workspace.tool_listing_completed(probe)
            ]
            if not pending:
                return
            returncode = process.poll()
            if returncode is not None:
                raise RuntimeError(
                    "Claude exited before MCP tool discovery completed; "
                    f"returncode={returncode} pending={pending}"
                )
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError(
                    "timed out waiting for Claude MCP tool discovery; "
                    f"pending={pending}"
                )
            time.sleep(min(self._ready_poll_interval_seconds, remaining))

    @staticmethod
    def _send_prompt(
        process: subprocess.Popen[str],
        prompt: str,
    ) -> None:
        if process.stdin is None:
            raise RuntimeError("Claude MCP streaming stdin is unavailable")
        message = {
            "type": "user",
            "message": {
                "role": "user",
                "content": prompt,
            },
            "parent_tool_use_id": None,
        }
        process.stdin.write(
            json.dumps(message, separators=(",", ":")) + "\n"
        )
        process.stdin.flush()
        process.stdin.close()
        process.stdin = None

    def _terminate_owned_group(
        self,
        process: subprocess.Popen[str],
    ) -> tuple[str, str, bool]:
        self._signal_group(process.pid, signal.SIGTERM)
        try:
            stdout, stderr = process.communicate(
                timeout=self._shutdown_timeout_seconds
            )
            return stdout, stderr, False
        except subprocess.TimeoutExpired:
            self._signal_group(process.pid, signal.SIGKILL)
            stdout, stderr = process.communicate(
                timeout=self._shutdown_timeout_seconds
            )
            return stdout, stderr, True

    @staticmethod
    def _signal_group(process_group_id: int, selected_signal: int) -> None:
        try:
            os.killpg(process_group_id, selected_signal)
        except ProcessLookupError:
            return
