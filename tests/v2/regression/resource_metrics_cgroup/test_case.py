from __future__ import annotations

import tempfile
import unittest
from pathlib import Path

from tests.v2.common.core import TestCaseInputs
from tests.v2.regression.resource_metrics_cgroup.case import (
    ResourceMetricsCgroupCase,
)


class ResourceMetricsCgroupContractTest(unittest.TestCase):
    def test_patch_keeps_required_mode_and_delegated_service_root(self) -> None:
        with tempfile.TemporaryDirectory() as directory:
            inputs = TestCaseInputs(
                repo=Path("/repo"),
                bin_dir=Path("target/release"),
                work_dir=Path(directory),
            )
            case = ResourceMetricsCgroupCase(inputs)

            patch = case._operator_patch()

            self.assertIn('mode = "cgroup-v2"', patch)
            self.assertIn("/sys/fs/cgroup/system.slice/", patch)
            self.assertIn("finalization_timeout_ms = 1000", patch)


if __name__ == "__main__":
    unittest.main()
