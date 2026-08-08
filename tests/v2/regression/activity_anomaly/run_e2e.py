#!/usr/bin/env python3

from __future__ import annotations

import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))

from tests.v2.common.core import TestCaseInputs, TestCase, TestResult, TestStatus
from tests.v2.common.runner import TestDefinition, run_one, TestingContextSingleton


class ActivityAnomalyCase(TestCase):
    def __init__(self, inputs: TestCaseInputs):
        self._inputs = inputs

    def run(self, test_context: TestingContextSingleton) -> TestResult:
        manifest = (
            self._inputs.repo
            / "examples/plugins/wit-component/activity-anomaly/Cargo.toml"
        )
        command = [
            "cargo",
            "test",
            "--manifest-path",
            str(manifest),
            "matching_activity_builds_alert_drafts",
        ]
        completed = subprocess.run(
            command,
            cwd=self._inputs.repo,
            capture_output=True,
            text=True,
            check=False,
        )

        test_context.output.command_output(completed.stdout, completed.stderr)
        if completed.returncode != 0:
            return TestResult(
                TestStatus.FAILED,
                f"activity-anomaly rule test exited with {completed.returncode}",
            )

        if "test tests::matching_activity_builds_alert_drafts ... ok" not in (
            completed.stdout + completed.stderr
        ):
            return TestResult(
                TestStatus.FAILED,
                "activity-anomaly rule test omitted passing evidence",
            )

        return TestResult(
            TestStatus.PASSED,
            "matching request, response, and command activity built alerts",
        )


TEST_DEFINITION = TestDefinition(
    name="plugin_activity_anomaly",
    description="Verify matching activity-anomaly rules build alert drafts",
    build_case=ActivityAnomalyCase,
)


if __name__ == "__main__":
    raise SystemExit(run_one(TEST_DEFINITION, REPO))
