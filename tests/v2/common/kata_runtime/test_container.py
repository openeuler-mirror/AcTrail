from __future__ import annotations

import json
import unittest
from pathlib import Path
from typing import Sequence

from tests.v2.common.kata_runtime.capabilities import CtrCapabilities
from tests.v2.common.kata_runtime.container import KataTestContainer
from tests.v2.common.kata_runtime.process import CommandResult
from tests.v2.common.kata_runtime.requirements import (
    KataCreateSpec,
    PreparePolicy,
    RequirementCheck,
    ResolvedImage,
)


class FakeImageRequirement:
    def __init__(self) -> None:
        self.ensure_policies: list[PreparePolicy] = []
        self.refresh_reasons: list[str] = []

    def ensure(self, policy: PreparePolicy) -> ResolvedImage:
        self.ensure_policies.append(policy)
        return ResolvedImage(
            reference="example.test/actrail-workload@sha256:1111",
            digest="sha256:1111",
        )

    def refresh(self, reason: str) -> ResolvedImage:
        self.refresh_reasons.append(reason)
        return ResolvedImage(
            reference="example.test/actrail-workload@sha256:2222",
            digest="sha256:2222",
        )


class PassingRequirements:
    name_prefix = "lifecycle"
    prepare_policy = PreparePolicy.MISSING

    def __init__(self) -> None:
        self.image = FakeImageRequirement()

    def validate_static(self) -> None:
        return None

    def create_spec(self, image: ResolvedImage) -> KataCreateSpec:
        return KataCreateSpec(
            namespace="default",
            runtime="io.containerd.kata332.v2",
            runtime_config=Path("/etc/kata/configuration.toml"),
            image=image,
            command=("/bin/sh", "-c", "sleep 600"),
            uid=1000,
            gid=39000,
            ready_timeout_seconds=1,
        )

    def validate_running(
        self,
        container: KataTestContainer,
    ) -> RequirementCheck:
        if not container.is_running():
            return RequirementCheck.not_ready(
                "Kata task is not running",
                refreshable=False,
            )
        return RequirementCheck.ready_check()

    def refresh(self, reason: str) -> ResolvedImage:
        return self.image.refresh(reason)


class RefreshThenPassRequirements(PassingRequirements):
    prepare_policy = PreparePolicy.REFRESH_INVALID

    def __init__(self) -> None:
        super().__init__()
        self._checks = [
            RequirementCheck.not_ready(
                "workload package version is stale",
                refreshable=True,
            ),
            RequirementCheck.ready_check(),
        ]

    def validate_running(
        self,
        container: KataTestContainer,
    ) -> RequirementCheck:
        if not container.is_running():
            return RequirementCheck.not_ready(
                "Kata task is not running",
                refreshable=False,
            )
        return self._checks.pop(0)


class NeverReadyRequirements(RefreshThenPassRequirements):
    def __init__(self) -> None:
        super().__init__()
        self._checks = [
            RequirementCheck.not_ready(
                "workload package version is stale",
                refreshable=True,
            ),
            RequirementCheck.not_ready(
                "refreshed workload still lacks python3",
                refreshable=True,
            ),
        ]


class NonRefreshableRequirements(PassingRequirements):
    prepare_policy = PreparePolicy.REFRESH_INVALID

    def validate_running(
        self,
        container: KataTestContainer,
    ) -> RequirementCheck:
        self.assert_container_running = container.is_running()
        return RequirementCheck.not_ready(
            "KVM access was denied",
            refreshable=False,
        )


class FakeCtrRunner:
    """Small in-memory adapter for the external ctr command seam."""

    def __init__(self) -> None:
        self.containers: dict[str, dict[str, str]] = {}
        self.tasks: set[str] = set()
        self.host_processes: dict[int, tuple[int, str]] = {}
        self.leak_host_processes = False
        self.started_images: list[str] = []
        self.exec_commands: list[tuple[str, ...]] = []
        self.started_exec_commands: list[tuple[str, ...]] = []

    def run(
        self,
        argv: Sequence[str],
        *,
        timeout: float | None = None,
    ) -> CommandResult:
        command = tuple(str(value) for value in argv)
        del timeout

        if "run" in command:
            run_index = command.index("run")
            run_args = command[run_index + 1 :]
            image_index = next(
                index
                for index, value in enumerate(run_args)
                if value.startswith("example.test/actrail-workload@")
            )
            container_id = run_args[image_index + 1]
            labels: dict[str, str] = {}
            for index, value in enumerate(run_args):
                if value == "--label":
                    key, label_value = run_args[index + 1].split("=", 1)
                    labels[key] = label_value
            self.containers[container_id] = labels
            self.tasks.add(container_id)
            self.host_processes = {
                3001: (1, f"containerd-shim-kata-v2 -id {container_id}"),
                3002: (3001, "/usr/bin/stratovirt --api-channel fd=3"),
            }
            self.started_images.append(run_args[image_index])
            return CommandResult(command, 0, "", "")

        if "containers" in command and "info" in command:
            container_id = command[-1]
            labels = self.containers.get(container_id)
            if labels is None:
                return CommandResult(command, 1, "", "container not found")
            return CommandResult(
                command,
                0,
                json.dumps({"ID": container_id, "Labels": labels}),
                "",
            )

        if command[3:5] == ("tasks", "list"):
            rows = "".join(
                f"{container_id} 1234 RUNNING\n"
                for container_id in sorted(self.tasks)
            )
            return CommandResult(
                command,
                0,
                "TASK PID STATUS\n" + rows,
                "",
            )

        if "tasks" in command and "exec" in command:
            self.exec_commands.append(command)
            return CommandResult(command, 0, "uid=123 gid=456\n", "")

        if "tasks" in command and "rm" in command:
            self.tasks.discard(command[-1])
            if not self.leak_host_processes:
                self.host_processes.clear()
            return CommandResult(command, 0, "", "")

        if "containers" in command and "rm" in command:
            self.containers.pop(command[-1], None)
            return CommandResult(command, 0, "", "")

        if command[:2] == ("ps", "-eo"):
            rows = "".join(
                f"{pid} {ppid} {arguments}\n"
                for pid, (ppid, arguments) in sorted(self.host_processes.items())
            )
            return CommandResult(command, 0, rows, "")

        return CommandResult(command, 1, "", f"unsupported fake command: {command}")

    def start(
        self,
        argv: Sequence[str],
        *,
        cwd: Path | None = None,
        environment: dict[str, str] | None = None,
    ) -> FakeManagedProcess:
        del cwd, environment
        command = tuple(argv)
        self.started_exec_commands.append(command)
        return FakeManagedProcess(command)


class FakeManagedProcess:
    def __init__(self, command: tuple[str, ...]) -> None:
        self.command = command
        self.terminated = False

    def wait(
        self,
        *,
        timeout: float | None = None,
        terminate_grace_seconds: float = 2,
    ) -> CommandResult:
        del timeout, terminate_grace_seconds
        return CommandResult(self.command, 0, "agent complete\n", "")

    def poll(self) -> int | None:
        return -15 if self.terminated else None

    def terminate(self, *, grace_seconds: float = 2) -> CommandResult:
        del grace_seconds
        self.terminated = True
        return CommandResult(self.command, -15, "", "")


class KataTestContainerLifecycleTest(unittest.TestCase):
    def test_context_exposes_running_task_then_removes_owned_resources(self) -> None:
        requirements = PassingRequirements()
        runner = FakeCtrRunner()
        capabilities = CtrCapabilities(
            run_user=True,
            exec_user=True,
            runtime_config_path=True,
        )

        with KataTestContainer(requirements, runner, capabilities) as container:
            container_id = container.container_id
            self.assertTrue(container.is_running())
            self.assertIn(container_id, runner.containers)
            self.assertIn(container_id, runner.tasks)

        self.assertNotIn(container_id, runner.containers)
        self.assertNotIn(container_id, runner.tasks)
        self.assertEqual(requirements.image.ensure_policies, [PreparePolicy.MISSING])

    def test_refreshable_runtime_mismatch_recreates_once_with_refreshed_image(
        self,
    ) -> None:
        requirements = RefreshThenPassRequirements()
        runner = FakeCtrRunner()
        capabilities = CtrCapabilities(True, True, True)

        with KataTestContainer(requirements, runner, capabilities) as container:
            self.assertTrue(container.is_running())
            self.assertEqual(
                runner.started_images,
                [
                    "example.test/actrail-workload@sha256:1111",
                    "example.test/actrail-workload@sha256:2222",
                ],
            )

        self.assertEqual(
            requirements.image.refresh_reasons,
            ["workload package version is stale"],
        )

    def test_exec_runs_argv_as_requested_numeric_identity(self) -> None:
        requirements = PassingRequirements()
        runner = FakeCtrRunner()
        capabilities = CtrCapabilities(True, True, True)

        with KataTestContainer(requirements, runner, capabilities) as container:
            result = container.exec(
                ("/usr/bin/id",),
                uid=123,
                gid=456,
                environment={"B": "two", "A": "one"},
            )

        self.assertEqual(result.returncode, 0)
        self.assertEqual(result.stdout, "uid=123 gid=456\n")
        exec_command = runner.exec_commands[-1]
        self.assertIn("--user", exec_command)
        self.assertIn("123:456", exec_command)
        self.assertEqual(
            exec_command[-4:],
            ("/usr/bin/env", "A=one", "B=two", "/usr/bin/id"),
        )

    def test_second_invalid_container_stops_after_one_refresh(self) -> None:
        requirements = NeverReadyRequirements()
        runner = FakeCtrRunner()

        with self.assertRaisesRegex(
            RuntimeError,
            "first=workload package version is stale; "
            "second=refreshed workload still lacks python3",
        ):
            KataTestContainer(
                requirements,
                runner,
                CtrCapabilities(True, True, True),
            ).start()

        self.assertEqual(len(requirements.image.refresh_reasons), 1)
        self.assertEqual(len(runner.started_images), 2)
        self.assertEqual(runner.containers, {})
        self.assertEqual(runner.tasks, set())

    def test_non_refreshable_failure_does_not_rebuild(self) -> None:
        requirements = NonRefreshableRequirements()
        runner = FakeCtrRunner()

        with self.assertRaisesRegex(RuntimeError, "KVM access was denied"):
            KataTestContainer(
                requirements,
                runner,
                CtrCapabilities(True, True, True),
            ).start()

        self.assertEqual(requirements.image.refresh_reasons, [])
        self.assertEqual(len(runner.started_images), 1)
        self.assertEqual(runner.containers, {})

    def test_body_exception_propagates_after_cleanup(self) -> None:
        runner = FakeCtrRunner()

        with self.assertRaisesRegex(ValueError, "scenario failed"):
            with KataTestContainer(
                PassingRequirements(),
                runner,
                CtrCapabilities(True, True, True),
            ):
                raise ValueError("scenario failed")

        self.assertEqual(runner.containers, {})
        self.assertEqual(runner.tasks, set())

    def test_close_refuses_to_remove_resource_with_foreign_owner_label(self) -> None:
        runner = FakeCtrRunner()
        container = KataTestContainer(
            PassingRequirements(),
            runner,
            CtrCapabilities(True, True, True),
        ).start()
        container_id = container.container_id
        runner.containers[container_id]["io.actrail.test.run"] = "another-run"

        with self.assertRaisesRegex(RuntimeError, "not owned by this run"):
            container.close()

        self.assertIn(container_id, runner.containers)
        self.assertIn(container_id, runner.tasks)

    def test_close_is_idempotent(self) -> None:
        runner = FakeCtrRunner()
        container = KataTestContainer(
            PassingRequirements(),
            runner,
            CtrCapabilities(True, True, True),
        ).start()

        container.close()
        container.close()

        self.assertEqual(runner.containers, {})
        self.assertEqual(runner.tasks, set())

    def test_diagnostics_reports_owned_task_shim_and_vmm(self) -> None:
        runner = FakeCtrRunner()
        container = KataTestContainer(
            PassingRequirements(),
            runner,
            CtrCapabilities(True, True, True),
        ).start()
        container_id = container.container_id

        diagnostic = container.diagnostics()
        container.close()

        self.assertIn(container_id, diagnostic)
        self.assertIn("containerd-shim-kata-v2", diagnostic)
        self.assertIn("stratovirt", diagnostic)

    def test_close_fails_when_owned_shim_or_vmm_remains(self) -> None:
        runner = FakeCtrRunner()
        runner.leak_host_processes = True
        container = KataTestContainer(
            PassingRequirements(),
            runner,
            CtrCapabilities(True, True, True),
            cleanup_timeout_seconds=0.001,
        ).start()

        with self.assertRaisesRegex(RuntimeError, "host process leak"):
            container.close()

        self.assertEqual(runner.containers, {})
        self.assertEqual(runner.tasks, set())
        self.assertIn("containerd-shim-kata-v2", container.diagnostics())
        container.close()

    def test_start_exec_uses_setpriv_when_ctr_exec_has_no_user_flag(self) -> None:
        runner = FakeCtrRunner()

        with KataTestContainer(
            PassingRequirements(),
            runner,
            CtrCapabilities(run_user=False, exec_user=False, runtime_config_path=True),
        ) as container:
            process = container.start_exec(
                ("/bin/sh", "/opt/actrail-xiaoo/workload.sh"),
                uid=1000,
                gid=39000,
            )
            result = process.wait(timeout=1)

        self.assertEqual(result.stdout, "agent complete\n")
        exec_command = runner.started_exec_commands[-1]
        setpriv = exec_command.index("/usr/bin/setpriv")
        self.assertEqual(
            exec_command[setpriv : setpriv + 8],
            (
                "/usr/bin/setpriv",
                "--reuid",
                "1000",
                "--regid",
                "39000",
                "--clear-groups",
                "--",
                "/bin/sh",
            ),
        )

    def test_close_terminates_unfinished_exec_before_removing_vm(self) -> None:
        runner = FakeCtrRunner()

        with KataTestContainer(
            PassingRequirements(),
            runner,
            CtrCapabilities(True, True, True),
        ) as container:
            process = container.start_exec(("/bin/sh", "-c", "sleep 600"))

        self.assertTrue(process.terminated)
        self.assertEqual(runner.containers, {})


if __name__ == "__main__":
    unittest.main()
