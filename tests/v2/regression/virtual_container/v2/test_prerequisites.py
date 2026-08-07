from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from tests.v2.common.config import TestCaseInputs
from tests.v2.common.test_case import TestStatus
from tests.v2.regression.virtual_container.v2.config import (
    VirtualContainerConfig,
)
from tests.v2.regression.virtual_container.v2.prerequisites import (
    VirtualContainerPrerequisites,
)


class SelectiveHostProbe:
    def command_path(self, name: str) -> Path | None:
        available = {
            "ctr",
            "kata-runtime",
            "containerd-shim-kata332-v2",
            "stratovirt",
        }
        return Path("/usr/bin") / name if name in available else None

    def kvm_available(self) -> bool:
        return True


class VirtualContainerPrerequisitesTest(unittest.TestCase):
    def test_missing_one_vmm_skips_only_its_backend(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prerequisites.") as raw_dir:
            root = Path(raw_dir)
            stratovirt_config = root / "stratovirt.toml"
            cloud_config = root / "cloud.toml"
            for path in (stratovirt_config, cloud_config):
                path.write_text(
                    "[agent.kata]\ndebug_console_enabled = true\n",
                    encoding="utf-8",
                )
            with patch.dict(
                "os.environ",
                {
                    "CTR_RUNTIME": "io.containerd.kata332.v2",
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": (
                        "stratovirt,cloud-hypervisor"
                    ),
                    "VIRTUAL_CONTAINER_E2E_STRATOVIRT_CONFIG": str(
                        stratovirt_config
                    ),
                    "VIRTUAL_CONTAINER_E2E_CLOUD_HYPERVISOR_CONFIG": str(
                        cloud_config
                    ),
                },
                clear=True,
            ):
                config = VirtualContainerConfig.from_environment(
                    TestCaseInputs(root, root / "bin", root / "work")
                )
            resolved = VirtualContainerPrerequisites(
                config,
                SelectiveHostProbe(),
            ).resolve_backends()

        self.assertIsNone(resolved["stratovirt"].problem)
        self.assertEqual(
            resolved["cloud-hypervisor"].problem.status,
            TestStatus.SKIPPED,
        )

    def test_missing_explicit_config_fails_before_unavailable_vmm_skip(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-prerequisites.") as raw_dir:
            root = Path(raw_dir)
            missing_config = root / "missing-cloud-hypervisor.toml"
            with patch.dict(
                "os.environ",
                {
                    "CTR_RUNTIME": "io.containerd.kata332.v2",
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": "cloud-hypervisor",
                    "VIRTUAL_CONTAINER_E2E_CLOUD_HYPERVISOR_CONFIG": str(
                        missing_config
                    ),
                },
                clear=True,
            ):
                config = VirtualContainerConfig.from_environment(
                    TestCaseInputs(root, root / "bin", root / "work")
                )
            resolved = VirtualContainerPrerequisites(
                config,
                SelectiveHostProbe(),
            ).resolve_backends()

        problem = resolved["cloud-hypervisor"].problem
        self.assertIsNotNone(problem)
        self.assertEqual(problem.status, TestStatus.FAILED)
        self.assertIn("configured Kata runtime config is missing", problem.message)


if __name__ == "__main__":
    unittest.main()
