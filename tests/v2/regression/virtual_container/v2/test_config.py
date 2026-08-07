from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.v2.common.core import TestCaseInputs
from tests.v2.common.kata_runtime.image import PullPolicy
from tests.v2.regression.virtual_container.v2.config import (
    VirtualContainerConfig,
)


class VirtualContainerConfigTest(unittest.TestCase):
    def test_parses_selected_backend_without_touching_filesystem(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-v2-config.") as raw_dir:
            root = Path(raw_dir)
            inputs = TestCaseInputs(root, root / "bin", root / "work")
            missing_config = root / "not-created.toml"
            with patch.dict(
                "os.environ",
                {
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": "stratovirt",
                    "VIRTUAL_CONTAINER_E2E_STRATOVIRT_CONFIG": str(
                        missing_config
                    ),
                    "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY": "never",
                    "VIRTUAL_CONTAINER_E2E_SETTLE_SECONDS": "0",
                },
                clear=True,
            ):
                config = VirtualContainerConfig.from_environment(inputs)

        self.assertEqual(config.backends, ("stratovirt",))
        self.assertEqual(config.image_pull_policy, PullPolicy.NEVER)
        self.assertEqual(config.runtime_config("stratovirt"), missing_config)
        self.assertEqual(config.settle_seconds, 0)

    def test_rejects_duplicate_backends(self) -> None:
        inputs = TestCaseInputs(Path("/repo"), Path("/bin"), Path("/work"))
        with patch.dict(
            "os.environ",
            {
                "VIRTUAL_CONTAINER_E2E_BACKENDS": "stratovirt,stratovirt",
            },
            clear=True,
        ):
            with self.assertRaisesRegex(ValueError, "must not contain duplicates"):
                VirtualContainerConfig.from_environment(inputs)


if __name__ == "__main__":
    unittest.main()
