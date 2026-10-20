from __future__ import annotations

import json
import logging
import os
import secrets
import shutil
import tempfile
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime, CommandResult
from tests.v2.common.core import AgentBinaryNotFoundError, TestOutput
from tests.v2.common.process import CommandTimeoutError, SubprocessRunner

from .config import ProbeDcodeLLMConfig


class ProbeDcodeLLMTask:
    def __init__(
        self, config: ProbeDcodeLLMConfig, runtime: ActrailRuntime, output: TestOutput
    ):
        self._config = config
        self._runtime = runtime
        self._output = output
        self._runner = SubprocessRunner(
            terminate_grace_seconds=config.command_timeout_seconds
        )
        self.marker = f"A{secrets.token_hex(5)}"
        discovered = shutil.which("dcode")
        if not discovered:
            raise AgentBinaryNotFoundError("dcode executable not found in PATH")
        self._binary = Path(discovered)

    def check_availability(self) -> bool:
        marker = f"A{secrets.token_hex(5)}"
        try:
            result = self._run_agent(
                self._agent_command(marker),
                echo=False,
            )
        except (OSError, CommandTimeoutError):
            return False
        return result.returncode == 0 and marker in result.stdout

    def run(self) -> CommandResult:
        return self._run_agent(
            self._runtime.control_command(
                "launch", "--", *self._agent_command(self.marker)
            ),
        )

    def _run_agent(
        self, command: list[Path | str], *, echo: bool = True
    ) -> CommandResult:
        work_dir = Path(
            tempfile.mkdtemp(prefix="dcode-invocation-", dir=self._config.work_dir)
        ).resolve()
        try:
            result = self._runner.run(
                [str(argument) for argument in command],
                timeout=self._config.launch_timeout_seconds,
                environment=os.environ.copy(),
                cwd=work_dir,
            )
            if echo:
                self._output.command_output(result.stdout, result.stderr)
            return CommandResult(
                result.argv, result.returncode, result.stdout, result.stderr
            )
        finally:
            self._cleanup_threads(work_dir)
            try:
                shutil.rmtree(work_dir)
            except OSError as error:
                logging.getLogger(__name__).warning(
                    "dcode invocation directory cleanup failed for %s: %s",
                    work_dir, error,
                )

    def _cleanup_threads(self, work_dir: Path) -> None:
        try:
            listing = self._runtime.run_checked(
                [self._binary, "threads", "list", "--cwd", work_dir, "--json"],
                echo=False,
            )
            for thread in json.loads(listing.stdout)["data"]:
                thread_id = thread["thread_id"]
                try:
                    self._runtime.run_checked(
                        [self._binary, "threads", "delete", thread_id, "--json"],
                        echo=False,
                    )
                except Exception as error:
                    logging.getLogger(__name__).warning(
                        "dcode thread cleanup failed for %s (cwd=%s): %s",
                        thread_id, work_dir, error,
                    )
        except Exception as error:
            logging.getLogger(__name__).warning(
                "dcode thread listing for cleanup failed (cwd=%s): %s",
                work_dir, error,
            )

    def _agent_command(self, marker: str) -> list[Path | str]:
        return [
            self._binary,
            "-n",
            f'Reply with exactly "{marker}" and nothing else. Do not use tools.',
            "-q",
            "--no-mcp",
            "--timeout",
            str(self._config.launch_timeout_seconds),
        ]
