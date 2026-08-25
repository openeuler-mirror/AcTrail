from __future__ import annotations

import sys
import unittest

from tests.v2.common.kata_runtime.process import (
    CommandTimeoutError,
    SubprocessRunner,
)


class ManagedProcessTest(unittest.TestCase):
    def test_wait_for_output_observes_marker_and_preserves_captured_streams(
        self,
    ) -> None:
        process = SubprocessRunner().start(
            [
                sys.executable,
                "-c",
                "import time; "
                "print('booting', flush=True); "
                "print('gateway ready gateway_id=17', flush=True); "
                "time.sleep(30)",
            ]
        )

        process.wait_for_output("gateway ready gateway_id=", timeout=1)
        result = process.terminate(grace_seconds=0.2)

        self.assertIn("booting", result.stdout)
        self.assertIn("gateway ready gateway_id=17", result.stdout)

    def test_wait_for_output_reports_early_exit_with_diagnostic(self) -> None:
        process = SubprocessRunner().start(
            [
                sys.executable,
                "-c",
                "import sys; print('bind failed', file=sys.stderr, flush=True)",
            ]
        )

        with self.assertRaisesRegex(RuntimeError, "bind failed"):
            process.wait_for_output("gateway ready gateway_id=", timeout=1)

    def test_timeout_terminates_process_group_and_keeps_output(self) -> None:
        process = SubprocessRunner().start(
            [
                sys.executable,
                "-c",
                "import time; print('READY', flush=True); time.sleep(30)",
            ]
        )

        with self.assertRaises(CommandTimeoutError) as caught:
            process.wait(timeout=0.1, terminate_grace_seconds=0.2)

        self.assertIn("READY", caught.exception.result.stdout)
        self.assertIsNotNone(process.poll())

    def test_synchronous_timeout_uses_the_same_process_group_cleanup(self) -> None:
        with self.assertRaises(CommandTimeoutError) as caught:
            SubprocessRunner().run(
                [
                    sys.executable,
                    "-c",
                    "import time; print('SYNC_READY', flush=True); time.sleep(30)",
                ],
                timeout=0.1,
            )

        self.assertIn("SYNC_READY", caught.exception.result.stdout)


if __name__ == "__main__":
    unittest.main()
