from __future__ import annotations

import io
import unittest
from unittest.mock import Mock

from tests.v2.common.output import CaseProgressReporter
from tests.v2.common.runner import _dependency_skip_result
from tests.v2.common.test_case import TestResult, TestStatus
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2.run_e2e import (
    TEST_DEFINITION as CONCURRENCY_DEFINITION,
)


class VirtualContainerProgressTest(unittest.TestCase):
    def test_compact_console_uses_step_without_verbose_message(self) -> None:
        console = Mock()
        log_stream = io.StringIO()
        log = Mock()
        log.line.side_effect = lambda value: log_stream.write(value + "\n")
        reporter = CaseProgressReporter(console, log, detailed=False)

        reporter.report("cloud.data.combo", "running in reusable data VM")

        console.progress_update.assert_called_once_with("cloud.data.combo")
        self.assertEqual(
            log_stream.getvalue(),
            "→ cloud.data.combo: running in reusable data VM\n",
        )

    def test_concurrency_skip_follows_selected_virtual_container_skip(self) -> None:
        result = _dependency_skip_result(
            CONCURRENCY_DEFINITION,
            {
                "virtual_container": TestResult(
                    TestStatus.SKIPPED,
                    "KVM runtime acceptance was not run",
                )
            },
        )

        self.assertIsNotNone(result)
        assert result is not None
        self.assertEqual(result.status, TestStatus.SKIPPED)
        self.assertIn("virtual_container", result.message)

    def test_concurrency_runs_when_virtual_container_was_not_selected(self) -> None:
        result = _dependency_skip_result(CONCURRENCY_DEFINITION, {})

        self.assertIsNone(result)


if __name__ == "__main__":
    unittest.main()
