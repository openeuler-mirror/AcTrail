from __future__ import annotations

import base64
import binascii
import re
import secrets
import shlex
from collections.abc import Sequence

from .process import CommandResult, CommandRunner


_SAFE_CONTAINER_ID = re.compile(r"^[A-Za-z0-9][A-Za-z0-9._-]*$")
_ANSI_CONTROL = re.compile(r"\x1b\[[0-?]*[ -/]*[@-~]")


class GuestConsole:
    """Runs bounded root commands through Kata's interactive debug console."""

    def __init__(
        self,
        runner: CommandRunner,
        *,
        kata_runtime: str = "kata-runtime",
        script_command: str = "script",
    ) -> None:
        self._runner = runner
        self._kata_runtime = kata_runtime
        self._script_command = script_command

    def execute(
        self,
        container_id: str,
        commands: Sequence[str],
        *,
        timeout: float,
    ) -> CommandResult:
        _validate_container_id(container_id)
        if timeout <= 0:
            raise ValueError("guest console timeout must be positive")
        if not commands or any(
            not command or "\x00" in command for command in commands
        ):
            raise ValueError("guest console requires non-empty commands without NUL")
        console_command = shlex.join(
            [self._kata_runtime, "exec", container_id]
        )
        argv = [
            self._script_command,
            "-qec",
            console_command,
            "/dev/null",
        ]
        # kata-runtime opens a root debug shell without a login environment.
        # AcTrail's operator config contains user-relative plugin paths, so
        # make HOME deterministic before running any viewer command.
        input_text = "\n".join(["export HOME=/root", *commands, "exit", ""])
        result = self._runner.run(
            argv,
            timeout=timeout,
            input_text=input_text,
        )
        return CommandResult(
            result.argv,
            result.returncode,
            result.stdout.replace("\r", ""),
            result.stderr.replace("\r", ""),
        )

    def capture(
        self,
        container_id: str,
        command: str,
        *,
        timeout: float,
    ) -> CommandResult:
        if not command or "\x00" in command:
            raise ValueError(
                "guest capture command must be non-empty and contain no NUL"
            )
        token = secrets.token_hex(12)
        payload_marker = f"__ACTRAIL_GUEST_PAYLOAD_{token}__"
        end = f"__ACTRAIL_GUEST_END_{token}__"
        temporary = f"/tmp/.actrail-guest-{token}"
        framed_command = (
            f"__actrail_tmp={temporary}; "
            f"( {command} ) >\"$__actrail_tmp\" 2>&1; "
            "__actrail_rc=$?; "
            f"printf '{payload_marker}:'; "
            "base64 -w0 \"$__actrail_tmp\"; "
            f"printf '\\n{end}:%s\\n' \"$__actrail_rc\"; "
            "rm -f \"$__actrail_tmp\""
        )
        console = self.execute(
            container_id,
            (framed_command,),
            timeout=timeout,
        )
        lines = console.stdout.splitlines()
        cleaned_lines = [_ANSI_CONTROL.sub("", line).strip() for line in lines]
        payload_lines = [
            line for line in cleaned_lines if line.startswith(payload_marker + ":")
        ]
        end_lines = [
            line
            for line in cleaned_lines
            if re.fullmatch(re.escape(end) + r":[0-9]+", line)
        ]
        if not payload_lines or not end_lines:
            raise RuntimeError(
                "guest console output omitted command capture markers: "
                + (console.diagnostic or "no diagnostic output")
            )
        payload_line = payload_lines[-1]
        end_line = end_lines[-1]
        try:
            returncode = int(end_line.removeprefix(end + ":"))
        except ValueError as error:
            raise RuntimeError(
                f"guest console returned an invalid command status: {end_line}"
            ) from error
        encoded_payload = payload_line.removeprefix(payload_marker + ":")
        try:
            payload = base64.b64decode(
                encoded_payload,
                validate=True,
            ).decode("utf-8")
        except (binascii.Error, UnicodeDecodeError) as error:
            raise RuntimeError(
                "guest console returned an invalid encoded command payload"
            ) from error
        return CommandResult(
            console.argv,
            returncode,
            payload,
            console.stderr,
        )


def _validate_container_id(container_id: str) -> None:
    if not _SAFE_CONTAINER_ID.fullmatch(container_id):
        raise ValueError(f"unsafe Kata container ID: {container_id}")
