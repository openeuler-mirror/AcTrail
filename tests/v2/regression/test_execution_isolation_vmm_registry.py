from __future__ import annotations

import subprocess
import shutil
import sys
import unittest
from pathlib import Path


REPO = Path(__file__).resolve().parents[3]
RUNNER = REPO / "tests/v2/regression/test_all.py"
PYTHON = shutil.which("python3.11") or sys.executable


class ExecutionIsolationVmmRegistryTest(unittest.TestCase):
    def test_stratovirt_case_is_available_through_the_public_runner(self) -> None:
        result = subprocess.run(
            (
                PYTHON,
                str(RUNNER),
                "--no-profile",
                "--list",
                "--case",
                "execution_isolation_stratovirt",
            ),
            cwd=REPO,
            check=False,
            capture_output=True,
            text=True,
        )

        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "execution_isolation_stratovirt:",
            result.stdout,
        )

    def test_firecracker_case_is_available_through_the_public_runner(self) -> None:
        result = subprocess.run(
            (
                PYTHON,
                str(RUNNER),
                "--no-profile",
                "--list",
                "--case",
                "execution_isolation_firecracker",
            ),
            cwd=REPO,
            check=False,
            capture_output=True,
            text=True,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("execution_isolation_firecracker:", result.stdout)


if __name__ == "__main__":
    unittest.main()
