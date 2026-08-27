from __future__ import annotations

import os
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tests.v2.common.core import TestCaseInputs
from tests.v2.common.process import CommandResult
from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.scenario.setup import (
    CloudHypervisorScenarioSetup,
)
from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.config import (
    CloudHypervisorExecutionIsolationConfig,
)
from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.case import (
    CloudHypervisorExecutionIsolationCase,
)
from tests.v2.regression.execution_isolation_cloud_hypervisor.v2.prerequisites import (
    CloudHypervisorExecutionIsolationPrerequisites,
)
from tests.v2.regression.execution_isolation_firecracker.v2.config import (
    FirecrackerExecutionIsolationConfig,
)
from tests.v2.regression.execution_isolation_firecracker.v2.run_e2e import (
    TEST_DEFINITION as FIRECRACKER_TEST_DEFINITION,
)
from tests.v2.regression.execution_isolation_stratovirt.v2.config import (
    StratoVirtExecutionIsolationConfig,
)


class _RecordingRunner:
    def __init__(self) -> None:
        self.commands: list[tuple[str, ...]] = []

    def run(self, argv: tuple[str, ...], **_: object) -> CommandResult:
        command = tuple(str(value) for value in argv)
        self.commands.append(command)
        return CommandResult(command, 0, "", "")


class ExecutionIsolationVmmAdapterTest(unittest.TestCase):
    def test_firecracker_definition_builds_shared_kata_alert_case(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            with patch.dict(os.environ, {}, clear=True):
                case = FIRECRACKER_TEST_DEFINITION.build_case(inputs)

        self.assertIsInstance(case, CloudHypervisorExecutionIsolationCase)
        self.assertEqual(case._config.BACKEND, "firecracker")

    def test_firecracker_uses_its_kata_runtime_profile_and_vm_root(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            runtime_config = root / "configuration-firecracker.toml"
            with patch.dict(
                os.environ,
                {
                    "VIRTUAL_CONTAINER_E2E_FIRECRACKER_DATA_CONFIG": str(
                        runtime_config
                    )
                },
                clear=True,
            ):
                config = FirecrackerExecutionIsolationConfig.from_environment(
                    inputs
                )

        self.assertEqual(config.runtime_config, runtime_config)
        self.assertEqual(config.vm_root, Path("/run/vc/firecracker"))
        self.assertFalse(hasattr(config, "rootfs_image"))
        self.assertFalse(hasattr(config, "firecracker_binary"))

    def test_kata_alert_scenario_requires_guest_system_observer(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            manifest = root / "manifest.json"
            with patch.dict(
                os.environ,
                {
                    "EXECUTION_ISOLATION_CLOUD_HYPERVISOR_E2E_ARTIFACT_MANIFEST": str(
                        manifest
                    )
                },
                clear=True,
            ):
                config = CloudHypervisorExecutionIsolationConfig.from_environment(
                    inputs
                )
            deployment = SimpleNamespace(
                data_config=root / "configuration-data.toml",
                workload_image=config.image,
                xiaoo=root / "xiaoo",
            )
            with patch(
                "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
                "prerequisites.DeploymentArtifacts.load",
                return_value=deployment,
            ) as load:
                resolved, problem = CloudHypervisorExecutionIsolationPrerequisites(
                    config
                )._deployment()

        self.assertIs(resolved, deployment)
        self.assertIsNone(problem)
        self.assertTrue(load.call_args.kwargs["require_sandbox_observer"])

    def test_xiaoo_workload_container_remains_unprivileged(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            with patch.dict(os.environ, {}, clear=True):
                config = StratoVirtExecutionIsolationConfig.from_environment(
                    inputs
                )
            deployment = SimpleNamespace(
                data_config=root / "configuration-data.toml",
                workload_image=config.image,
                workload_bundle=root / "workload",
                guest_bundle=root / "guest",
            )
            setup = CloudHypervisorScenarioSetup(
                config,
                deployment,  # type: ignore[arg-type]
                _RecordingRunner(),  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )

            requirements = setup.requirements()

        self.assertEqual(requirements.uid, 1000)
        self.assertEqual(requirements.gid, 39000)
        self.assertFalse(requirements.privileged_without_host_devices)

    def test_firecracker_uses_manifest_workload_image_archive(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            profile_archive = root / "stale-profile-workload.tar"
            manifest_archive = root / "manifest-workload.tar"
            with patch.dict(
                os.environ,
                {
                    "EXECUTION_ISOLATION_FIRECRACKER_E2E_IMAGE_ARCHIVE": str(
                        profile_archive
                    )
                },
                clear=True,
            ):
                config = FirecrackerExecutionIsolationConfig.from_environment(
                    inputs
                )
            deployment = SimpleNamespace(
                data_config=root / "configuration-data.toml",
                workload_image=config.image,
                workload_image_archive=manifest_archive,
                workload_image_archive_sha256="a" * 64,
            )
            setup = CloudHypervisorScenarioSetup(
                config,
                deployment,  # type: ignore[arg-type]
                _RecordingRunner(),  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )

            requirements = setup.requirements()

        self.assertEqual(requirements.image.archive, manifest_archive)
        self.assertEqual(requirements.image.archive_sha256, "a" * 64)

    def test_firecracker_rejects_a_stale_profile_workload_archive(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            profile_archive = root / "stale-profile-workload.tar"
            manifest_archive = root / "manifest-workload.tar"
            with patch.dict(
                os.environ,
                {
                    "EXECUTION_ISOLATION_FIRECRACKER_E2E_ARTIFACT_MANIFEST": str(
                        root / "manifest.json"
                    ),
                    "EXECUTION_ISOLATION_FIRECRACKER_E2E_IMAGE_ARCHIVE": str(
                        profile_archive
                    ),
                },
                clear=True,
            ):
                config = FirecrackerExecutionIsolationConfig.from_environment(
                    inputs
                )
            deployment = SimpleNamespace(
                data_config=root / "configuration-data.toml",
                workload_image=config.image,
                workload_image_archive=manifest_archive,
                xiaoo=root / "xiaoo",
            )
            with patch(
                "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
                "prerequisites.DeploymentArtifacts.load",
                return_value=deployment,
            ):
                resolved, problem = CloudHypervisorExecutionIsolationPrerequisites(
                    config
                )._deployment()

        self.assertIsNone(resolved)
        self.assertIsNotNone(problem)
        assert problem is not None
        self.assertIn("workload image archive", problem.message)

    def test_stratovirt_uses_native_af_vsock(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            with patch.dict(os.environ, {}, clear=True):
                config = StratoVirtExecutionIsolationConfig.from_environment(
                    inputs
                )
            runner = _RecordingRunner()
            setup = CloudHypervisorScenarioSetup(
                config,
                None,  # type: ignore[arg-type]
                runner,  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )
            setup.write_gateway_config(None, 19472)

        command = runner.commands[-1]
        self.assertIn("native", command)
        self.assertIn("4294967295", command)
        self.assertIn("43182", command)
        self.assertNotIn("--socket-path", command)

    def test_cloud_hypervisor_uses_runtime_uds(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            with patch.dict(os.environ, {}, clear=True):
                config = CloudHypervisorExecutionIsolationConfig.from_environment(
                    inputs
                )
            runner = _RecordingRunner()
            setup = CloudHypervisorScenarioSetup(
                config,
                None,  # type: ignore[arg-type]
                runner,  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )
            socket_path = root / "vm/clh.sock_43182"
            setup.write_gateway_config(socket_path, 19472)

        command = runner.commands[-1]
        self.assertIn("cloud-hypervisor", command)
        self.assertIn("--socket-path", command)
        self.assertIn(str(socket_path), command)
        self.assertNotIn("--uds-path", command)
        self.assertNotIn("--port", command)

    def test_firecracker_uses_base_uds_and_concrete_vsock_port(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            inputs = TestCaseInputs(root, root, root)
            with patch.dict(os.environ, {}, clear=True):
                config = FirecrackerExecutionIsolationConfig.from_environment(
                    inputs
                )
            runner = _RecordingRunner()
            setup = CloudHypervisorScenarioSetup(
                config,
                None,  # type: ignore[arg-type]
                runner,  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )
            uds_path = root / "vm/vsock.sock"
            setup.write_gateway_config(uds_path, 19472)

        command = runner.commands[-1]
        self.assertIn("firecracker", command)
        self.assertIn("--uds-path", command)
        self.assertIn(str(uds_path), command)
        self.assertIn("43182", command)

if __name__ == "__main__":
    unittest.main()
