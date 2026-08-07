from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.v2.common.config import TestCaseInputs
from tests.v2.common.kata_runtime.image import PullPolicy
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2.config import (
    VirtualContainerXiaooConcurrencyConfig,
)


class VirtualContainerXiaooConcurrencyConfigTest(unittest.TestCase):
    def test_uses_data_profile_and_never_pull_by_default(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-xiaoo-config.") as raw_dir:
            root = Path(raw_dir)
            runtime_config = root / "data.toml"
            xiaoo = root / "xiaoo"
            with patch.dict(
                "os.environ",
                {
                    "CTR_RUNTIME": "io.containerd.kata332.v2",
                    "XIAOO_E2E_BINARY": str(xiaoo),
                    "VIRTUAL_CONTAINER_E2E_STRATOVIRT_DATA_CONFIG": str(
                        runtime_config
                    ),
                    "VIRTUAL_CONTAINER_E2E_IMAGE_PULL_POLICY": "never",
                },
                clear=True,
            ):
                config = VirtualContainerXiaooConcurrencyConfig.from_environment(
                    TestCaseInputs(root, root / "bin", root / "work")
                )

        self.assertEqual(config.backend, "stratovirt")
        self.assertEqual(config.runtime_config, runtime_config)
        self.assertEqual(config.xiaoo_binary, xiaoo)
        self.assertEqual(config.image_pull_policy, PullPolicy.NEVER)


if __name__ == "__main__":
    unittest.main()
