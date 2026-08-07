from __future__ import annotations

import unittest
from collections.abc import Sequence

from tests.v2.common.kata_runtime.capabilities import CtrCapabilities
from tests.v2.common.kata_runtime.process import CommandResult


class HelpRunner:
    def __init__(self) -> None:
        self.commands: list[tuple[str, ...]] = []

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
    ) -> CommandResult:
        command = tuple(argv)
        self.commands.append(command)
        del timeout
        if command == ("ctr", "run", "--help"):
            return CommandResult(
                command,
                0,
                "OPTIONS:\n  --runtime-config-path value\n  --uidmap value\n",
                "",
            )
        if command == ("ctr", "tasks", "exec", "--help"):
            return CommandResult(
                command,
                0,
                "OPTIONS:\n  --exec-id value\n  --user value\n",
                "",
            )
        return CommandResult(command, 1, "", "unexpected command")


class CtrCapabilitiesTest(unittest.TestCase):
    def test_detects_run_and_exec_flags_independently(self) -> None:
        runner = HelpRunner()

        capabilities = CtrCapabilities.detect(runner)

        self.assertFalse(capabilities.run_user)
        self.assertTrue(capabilities.exec_user)
        self.assertTrue(capabilities.runtime_config_path)
        self.assertEqual(
            runner.commands,
            [
                ("ctr", "run", "--help"),
                ("ctr", "tasks", "exec", "--help"),
            ],
        )


if __name__ == "__main__":
    unittest.main()
