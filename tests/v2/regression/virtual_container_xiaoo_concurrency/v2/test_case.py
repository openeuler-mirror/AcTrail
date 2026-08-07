from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.v2.common.core import TestCaseInputs
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2.case import (
    VirtualContainerXiaooConcurrencyCase,
)
from tests.v2.regression.virtual_container_xiaoo_concurrency.v2.config import (
    VirtualContainerXiaooConcurrencyConfig,
)


class VirtualContainerXiaooConcurrencyCaseTest(unittest.TestCase):
    def test_valid_runtime_config_satisfies_prerequisites(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-xiaoo-case.") as raw_dir:
            root = Path(raw_dir)
            runtime_config = root / "data.toml"
            runtime_config.write_text(
                "[hypervisor.stratovirt]\n"
                "debug_console_enabled = true\n"
                "default_vcpus = 2\n",
                encoding="utf-8",
            )
            xiaoo = root / "xiaoo"
            xiaoo.write_text("xiaoo", encoding="utf-8")
            xiaoo.chmod(0o755)
            workload_bundle = root / "workload-bundle"
            workload_bundle.mkdir()
            (workload_bundle / "MANIFEST.sha256").write_text(
                "manifest",
                encoding="utf-8",
            )
            for relative in (
                "tests/support/llm-http-proxy/provider_proxy.py",
                "tests/v2/regression/virtual_container/"
                "validate-runtime-config.py",
            ):
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture", encoding="utf-8")
            with patch.dict(
                "os.environ",
                {
                    "CTR_RUNTIME": "io.containerd.kata332.v2",
                    "XIAOO_E2E_BINARY": str(xiaoo),
                    "WORKLOAD_BUNDLE_DIR": str(workload_bundle),
                    "VIRTUAL_CONTAINER_E2E_STRATOVIRT_DATA_CONFIG": str(
                        runtime_config
                    ),
                },
                clear=True,
            ):
                config = VirtualContainerXiaooConcurrencyConfig.from_environment(
                    TestCaseInputs(root, root / "bin", root / "work")
                )
            case = VirtualContainerXiaooConcurrencyCase(config)
            with patch(
                "tests.v2.regression.virtual_container_xiaoo_concurrency.v2."
                "case.shutil.which",
                return_value="/usr/bin/true",
            ), patch(
                "tests.v2.regression.virtual_container_xiaoo_concurrency.v2."
                "case.os.access",
                return_value=True,
            ):
                problem = case._prerequisite_problem(None)

        self.assertIsNone(problem)


if __name__ == "__main__":
    unittest.main()
