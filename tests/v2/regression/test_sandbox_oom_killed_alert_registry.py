from __future__ import annotations

import shutil
import subprocess
import sys
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "tests/v2/regression/test_all.py"
PYTHON = shutil.which("python3.11") or sys.executable


class SandboxOomKilledAlertRegistryTest(unittest.TestCase):
    def test_oom_killed_case_is_available_through_the_public_runner(self) -> None:
        result = subprocess.run(
            (
                PYTHON,
                str(RUNNER),
                "--no-profile",
                "--list",
                "--case",
                "sandbox_oom_killed_alert_host",
            ),
            cwd=REPO,
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("sandbox_oom_killed_alert_host:", result.stdout)


if __name__ == "__main__":
    unittest.main()
