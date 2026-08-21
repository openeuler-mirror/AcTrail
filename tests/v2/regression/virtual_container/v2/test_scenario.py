from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from tests.v2.common.core import TestCaseInputs, TestStatus
from tests.v2.common.kata_runtime.process import CommandResult

from tests.v2.regression.virtual_container.v2.config import VirtualContainerConfig
from tests.v2.regression.virtual_container.v2.scenario import VirtualContainerScenario


RELEASE_FILES = (
    "actraild",
    "actrailctl",
    "actrailviewer",
    "libactrail_tls_payload_probe_sync.so",
)


class RunCommandDiagnosticTest(unittest.TestCase):
    def _scenario(self, root: Path, result: CommandResult) -> VirtualContainerScenario:
        bin_dir = root / "bin"
        bin_dir.mkdir()
        for name in RELEASE_FILES:
            (bin_dir / name).write_bytes(name.encode())
        with patch.dict("os.environ", {}, clear=True):
            config = VirtualContainerConfig.from_environment(
                TestCaseInputs(root, bin_dir, root / "work")
            )
        scenario = VirtualContainerScenario(config, Mock())
        runner = Mock()
        runner.run.return_value = result
        scenario._runner = runner
        return scenario

    def _run(self, result: CommandResult) -> TestStatus:
        with tempfile.TemporaryDirectory(prefix="actrail-run-command.") as raw_dir:
            scenario = self._scenario(Path(raw_dir), result)
            return scenario._run_command(
                ["prepare-guest-bundle.sh"],
                environment={},
                timeout=5.0,
            )

    def test_failure_reports_the_script_diagnostic(self) -> None:
        outcome = self._run(
            CommandResult(
                argv=("prepare-guest-bundle.sh",),
                returncode=1,
                stdout="== prepare AcTrail guest bundle ==\n",
                stderr="FAIL: BUNDLE_SYSROOT is not a directory: /nope\n",
            )
        )

        self.assertEqual(outcome.status, TestStatus.FAILED)
        self.assertIn("exit status 1", outcome.message)
        self.assertIn(
            "FAIL: BUNDLE_SYSROOT is not a directory: /nope",
            outcome.message,
        )

    def test_failure_falls_back_to_stdout_when_stderr_is_empty(self) -> None:
        outcome = self._run(
            CommandResult(
                argv=("prepare-guest-bundle.sh",),
                returncode=2,
                stdout="missing command: readelf\n",
                stderr="",
            )
        )

        self.assertEqual(outcome.status, TestStatus.FAILED)
        self.assertIn("exit status 2", outcome.message)
        self.assertIn("missing command: readelf", outcome.message)

    def test_silent_failure_still_reports_the_exit_status(self) -> None:
        outcome = self._run(
            CommandResult(
                argv=("prepare-guest-bundle.sh",),
                returncode=3,
                stdout="",
                stderr="",
            )
        )

        self.assertEqual(outcome.status, TestStatus.FAILED)
        self.assertIn("exit status 3", outcome.message)

    def test_success_message_is_unchanged(self) -> None:
        outcome = self._run(
            CommandResult(
                argv=("prepare-guest-bundle.sh",),
                returncode=0,
                stdout="== prepare AcTrail guest bundle ==\n",
                stderr="",
            )
        )

        self.assertEqual(outcome.status, TestStatus.PASSED)
        self.assertEqual(outcome.message, "completed")


if __name__ == "__main__":
    unittest.main()
