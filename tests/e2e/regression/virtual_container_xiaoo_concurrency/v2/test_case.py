from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from tests.e2e.common.core import TestRunDirectories, TestStatus
from tests.e2e.regression.virtual_container_xiaoo_concurrency.v2.case import (
    VirtualContainerXiaooConcurrencyCase,
)


class VirtualContainerXiaooConcurrencyCaseTest(unittest.TestCase):
    def test_no_kvm_skips_before_artifact_resolution(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-xiaoo-no-kvm.") as raw_dir:
            root = Path(raw_dir)
            with patch.dict("os.environ", {}, clear=True):
                case = VirtualContainerXiaooConcurrencyCase(
                    TestRunDirectories(root, root / "bin", root / "work")
                )
            with patch(
                "tests.e2e.regression.virtual_container_xiaoo_concurrency.v2."
                "case.os.access",
                return_value=False,
            ), patch.object(case, "_resolve_deployment") as resolve_deployment:
                result = case.run(Mock())

        resolve_deployment.assert_not_called()
        self.assertEqual(result.status, TestStatus.SKIPPED)
        self.assertIn("/dev/kvm", result.message)

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
                "tests/e2e/common/llm-http-proxy/provider_proxy.py",
                "tests/e2e/regression/virtual_container/"
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
                case = VirtualContainerXiaooConcurrencyCase(
                    TestRunDirectories(root, root / "bin", root / "work")
                )
            with patch(
                "tests.e2e.regression.virtual_container_xiaoo_concurrency.v2."
                "case.shutil.which",
                return_value="/usr/bin/true",
            ), patch(
                "tests.e2e.regression.virtual_container_xiaoo_concurrency.v2."
                "case.os.access",
                return_value=True,
            ):
                problem = case._prerequisite_problem(None)

        self.assertIsNone(problem)


if __name__ == "__main__":
    unittest.main()
