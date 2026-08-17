from __future__ import annotations

import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

from tests.v2.common.core import TestCaseInputs, TestResult, TestStatus, effective_status

from tests.v2.regression.virtual_container.v2.case import VirtualContainerCase
from tests.v2.regression.virtual_container.v2.config import VirtualContainerConfig
from tests.v2.regression.virtual_container.v2.prerequisites import ResolvedBackend


class VirtualContainerCaseTest(unittest.TestCase):
    def test_without_kvm_skips_before_all_other_checks(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-auto-no-kvm.") as raw_dir:
            root = Path(raw_dir)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name in (
                "actraild",
                "actrailctl",
                "actrailviewer",
                "libactrail_tls_payload_probe_sync.so",
            ):
                (bin_dir / name).write_bytes(name.encode())
            with patch.dict("os.environ", {}, clear=True):
                config = VirtualContainerConfig.from_environment(
                    TestCaseInputs(root, bin_dir, root / "work")
                )
            context = Mock()
            with patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerScenario.run_contracts",
            ) as run_contracts, patch(
                "tests.v2.regression.virtual_container.v2.prerequisites."
                "LocalHostProbe.kvm_available",
                return_value=False,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerPrerequisites.release_problem",
            ) as release_problem:
                result = VirtualContainerCase(config).run(context)

        release_problem.assert_not_called()
        run_contracts.assert_not_called()
        self.assertEqual(result.status, TestStatus.SKIPPED)
        self.assertIn("/dev/kvm", result.message)

    def test_contract_scope_does_not_resolve_deployment_or_runtime(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-contracts.") as raw_dir:
            root = Path(raw_dir)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name in (
                "actraild",
                "actrailctl",
                "actrailviewer",
                "libactrail_tls_payload_probe_sync.so",
            ):
                (bin_dir / name).write_bytes(name.encode())
            with patch.dict(
                "os.environ",
                {"VIRTUAL_CONTAINER_E2E_SCOPE": "contracts"},
                clear=True,
            ):
                config = VirtualContainerConfig.from_environment(
                    TestCaseInputs(root, bin_dir, root / "work")
                )
            context = Mock()
            contract_result = TestResult(TestStatus.PASSED, "contracts")
            with patch(
                "tests.v2.regression.virtual_container.v2.prerequisites."
                "LocalHostProbe.kvm_available",
                return_value=True,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerScenario.run_contracts",
                return_value=contract_result,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerPrerequisites.resolve_deployment"
            ) as resolve_deployment:
                result = VirtualContainerCase(config).run(context)

        resolve_deployment.assert_not_called()
        self.assertEqual(result.status, TestStatus.SKIPPED)
        self.assertIn("KVM runtime acceptance was not run", result.message)
        self.assertIs(result.details["contracts"], contract_result)

    def test_unavailable_kvm_backend_skips_overall_runtime_acceptance(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-no-kvm.") as raw_dir:
            root = Path(raw_dir)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name in (
                "actraild",
                "actrailctl",
                "actrailviewer",
                "libactrail_tls_payload_probe_sync.so",
            ):
                (bin_dir / name).write_bytes(name.encode())
            with patch.dict(
                "os.environ",
                {
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": "cloud-hypervisor",
                    "VIRTUAL_CONTAINER_E2E_SCOPE": "all",
                },
                clear=True,
            ):
                config = VirtualContainerConfig.from_environment(
                    TestCaseInputs(root, bin_dir, root / "work")
                )
            context = Mock()
            contract_result = TestResult(TestStatus.PASSED, "contracts")
            backend_skip = TestResult(
                TestStatus.SKIPPED,
                "readable/writable /dev/kvm is unavailable",
            )
            resolved = ResolvedBackend(
                "cloud-hypervisor",
                None,
                None,
                backend_skip,
            )
            with patch(
                "tests.v2.regression.virtual_container.v2.prerequisites."
                "LocalHostProbe.kvm_available",
                return_value=True,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerScenario.run_contracts",
                return_value=contract_result,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerPrerequisites.resolve_deployment",
                return_value=(None, None),
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerPrerequisites.resolve_backends",
                return_value={"cloud-hypervisor": resolved},
            ):
                result = VirtualContainerCase(config).run(context)

        self.assertEqual(result.status, TestStatus.SKIPPED)
        self.assertIn("cloud-hypervisor", result.message)
        self.assertIs(result.details["cloud-hypervisor"], backend_skip)

    def test_one_backend_skip_does_not_hide_another_backend_success(self) -> None:
        with tempfile.TemporaryDirectory(prefix="actrail-mixed-backends.") as raw_dir:
            root = Path(raw_dir)
            bin_dir = root / "bin"
            bin_dir.mkdir()
            for name in (
                "actraild",
                "actrailctl",
                "actrailviewer",
                "libactrail_tls_payload_probe_sync.so",
            ):
                (bin_dir / name).write_bytes(name.encode())
            with patch.dict(
                "os.environ",
                {
                    "VIRTUAL_CONTAINER_E2E_BACKENDS": (
                        "stratovirt,cloud-hypervisor"
                    ),
                    "VIRTUAL_CONTAINER_E2E_SCOPE": "all",
                },
                clear=True,
            ):
                config = VirtualContainerConfig.from_environment(
                    TestCaseInputs(root, bin_dir, root / "work")
                )
            context = Mock()
            contract_result = TestResult(TestStatus.PASSED, "contracts")
            backend_pass = TestResult(TestStatus.PASSED, "stratovirt passed")
            backend_skip = TestResult(TestStatus.SKIPPED, "cloud unavailable")
            resolved = {
                "stratovirt": ResolvedBackend("stratovirt", root, root),
                "cloud-hypervisor": ResolvedBackend(
                    "cloud-hypervisor",
                    None,
                    None,
                    backend_skip,
                ),
            }
            with patch(
                "tests.v2.regression.virtual_container.v2.prerequisites."
                "LocalHostProbe.kvm_available",
                return_value=True,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerScenario.run_contracts",
                return_value=contract_result,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerPrerequisites.resolve_deployment",
                return_value=(None, None),
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerPrerequisites.resolve_backends",
                return_value=resolved,
            ), patch(
                "tests.v2.regression.virtual_container.v2.case."
                "VirtualContainerScenario.run_backend",
                return_value=backend_pass,
            ):
                result = VirtualContainerCase(config).run(context)

        self.assertEqual(result.status, TestStatus.COMPOSITE)
        self.assertEqual(effective_status(result), TestStatus.PASSED)
        self.assertIs(result.details["stratovirt"], backend_pass)
        self.assertIs(result.details["cloud-hypervisor"], backend_skip)


if __name__ == "__main__":
    unittest.main()
