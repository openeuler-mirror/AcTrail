from __future__ import annotations

import base64
import re
import unittest
from collections.abc import Mapping, Sequence
from pathlib import Path

from tests.v2.common.kata_runtime.guest import GuestConsole
from tests.v2.common.kata_runtime.process import CommandResult


class ConsoleRunner:
    def __init__(
        self,
        payload: str = '{"traces": []}\n',
        *,
        returncode: int = 0,
    ) -> None:
        self.input_text = ""
        self.payload = payload
        self.returncode = returncode

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
        cwd: Path | None = None,
        environment: Mapping[str, str] | None = None,
        input_text: str | None = None,
    ) -> CommandResult:
        del timeout, cwd, environment
        assert input_text is not None
        self.input_text = input_text
        payload_marker = re.search(
            r"printf '(__ACTRAIL_GUEST_PAYLOAD_[a-f0-9]+__):'",
            input_text,
        )
        end = re.search(r"\\n(__ACTRAIL_GUEST_END_[a-f0-9]+__):%s", input_text)
        assert payload_marker is not None and end is not None
        encoded = base64.b64encode(self.payload.encode()).decode()
        stdout = (
            "\x1b[?2004hroot@kata:/# __actrail_tmp=/tmp/.actrail-guest\r\n"
            "--output-format json traces ) >\"$__actrail_tmp\" 2>&1;\r\n"
            "wrapped interactive echo that is not command output\r\n"
            f"\x1b[?2004l{payload_marker.group(1)}:{encoded}\r\n"
            f"{end.group(1)}:{self.returncode}\r\n"
            "root@kata:/# exit\r\n"
        )
        return CommandResult(tuple(argv), 0, stdout, "")


class GuestConsoleTest(unittest.TestCase):
    def test_capture_ignores_wrapped_tty_noise_and_returns_guest_status(self) -> None:
        runner = ConsoleRunner()
        result = GuestConsole(runner).capture(
            "actrail-v2-data-1234",
            "/usr/local/bin/actrailviewer --output-format json traces",
            timeout=10,
        )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, '{"traces": []}\n')
        self.assertNotIn("\r", result.stdout)
        self.assertTrue(runner.input_text.startswith("export HOME=/root\n"))
        self.assertIn("base64 -w0", runner.input_text)

    def test_capture_preserves_multiline_output_exactly(self) -> None:
        runner = ConsoleRunner("line one\nline two\n")

        result = GuestConsole(runner).capture(
            "actrail-v2-data-1234",
            "journalctl -u actraild.service",
            timeout=10,
        )

        self.assertEqual(result.stdout, "line one\nline two\n")

    def test_capture_returns_guest_command_failure_status(self) -> None:
        runner = ConsoleRunner("viewer failed\n", returncode=17)

        result = GuestConsole(runner).capture(
            "actrail-v2-data-1234",
            "actrailviewer traces",
            timeout=10,
        )

        self.assertEqual(result.returncode, 17)
        self.assertEqual(result.stdout, "viewer failed\n")


if __name__ == "__main__":
    unittest.main()
