from __future__ import annotations

import base64
import io
import os
import socket
import subprocess
import tarfile
import tempfile
import unittest
from pathlib import Path
from types import SimpleNamespace
from unittest.mock import patch

from tests.v2.common.core import TestCaseInputs, TestStatus
from tests.v2.common.process import CommandResult, SubprocessRunner
from tests.v2.regression.execution_isolation_firecracker.v2.config import (
    FirecrackerExecutionIsolationConfig,
)

from .asset_bundle import CloudHypervisorAssetBundle
from .cloud_hypervisor import (
    CloudHypervisorSocketInventory,
    FirecrackerSocketInventory,
)
from .prerequisites import CloudHypervisorExecutionIsolationPrerequisites
from .scenario.runtime import CloudHypervisorExecutionIsolationScenario
from .scenario.setup import CloudHypervisorScenarioSetup
from .scenario.system_observer import GuestSystemSandboxObserver
from .scenario.transport import FirecrackerAssetTransport, GuestCoordination
from .scenario.verifier import CloudHypervisorAlertVerifier


class _RecordingRunner:
    def __init__(self, results: list[CommandResult] | None = None) -> None:
        self.results = list(results or [])
        self.calls: list[tuple[tuple[str, ...], dict[str, object]]] = []

    def run(self, argv: tuple[str, ...], **kwargs: object) -> CommandResult:
        command = tuple(str(value) for value in argv)
        self.calls.append((command, kwargs))
        if self.results:
            result = self.results.pop(0)
            return CommandResult(
                command,
                result.returncode,
                result.stdout,
                result.stderr,
            )
        return CommandResult(command, 0, "", "")


class _Identity:
    OOM_KILL_MARKER = "ACTRAIL_TEST_OOM_KILL_OK"

    @staticmethod
    def failure(message: str) -> str:
        return f"test identity: {message}"


class _GuestCapture:
    def __init__(self, results: list[CommandResult]) -> None:
        self.results = list(results)
        self.calls: list[tuple[str, str, float]] = []

    def capture(
        self,
        container_id: str,
        command: str,
        *,
        timeout: float,
    ) -> CommandResult:
        self.calls.append((container_id, command, timeout))
        return self.results.pop(0)


class _LocalVm:
    def __init__(self) -> None:
        self.runner = SubprocessRunner()
        self.calls: list[tuple[tuple[str, ...], dict[str, object]]] = []

    def exec(self, command: tuple[str, ...], **kwargs: object) -> CommandResult:
        normalized = tuple(str(value) for value in command)
        self.calls.append((normalized, kwargs))
        return self.runner.run(
            normalized,
            timeout=kwargs.get("timeout"),  # type: ignore[arg-type]
            input_text=kwargs.get("input_text"),  # type: ignore[arg-type]
        )


class SharedAlertRuntimeTest(unittest.TestCase):
    def test_named_root_pid_is_resolved_into_guest_system_namespace(self) -> None:
        guest = _GuestCapture(
            [CommandResult(("guest",), 0, "991\n", "")]
        )

        class CoordinationFile:
            def read_text(self, *, encoding: str) -> str:
                self.encoding = encoding
                return "17\n"

        class Coordination:
            def file(self, name: str) -> CoordinationFile:
                self.name = name
                return CoordinationFile()

        verifier = CloudHypervisorAlertVerifier(
            SimpleNamespace(
                ready_timeout_seconds=90,
                IDENTITY=_Identity,
            ),  # type: ignore[arg-type]
            SimpleNamespace(),  # type: ignore[arg-type]
            Path("/unused"),
            guest,  # type: ignore[arg-type]
        )
        vm = SimpleNamespace(container_id="owned-kata-vm")

        root_pid = verifier.read_root_pid(
            vm,  # type: ignore[arg-type]
            Coordination(),  # type: ignore[arg-type]
        )

        self.assertEqual(root_pid, 991)
        self.assertEqual(len(guest.calls), 1)
        self.assertEqual(guest.calls[0][0], "owned-kata-vm")
        self.assertEqual(guest.calls[0][2], 10)
        command = guest.calls[0][1]
        self.assertIn("target=17", command)
        self.assertIn("actrail-root", command)
        self.assertIn("NSpid:", command)
        self.assertIn("print $NF", command)

    def test_final_alert_timeout_reports_missing_categories_and_pid_candidates(
        self,
    ) -> None:
        records = [
            SimpleNamespace(
                category="sandbox.resource.high_cpu",
                gateway_id=1,
                sb_id=2,
                process=None,
            ),
            SimpleNamespace(
                category="sandbox.process.high_read",
                gateway_id=1,
                sb_id=2,
                process={
                    "pid": 991,
                    "start_time_ticks": 4432,
                    "executable_name_hex": "6163747261696c2d726f6f7400000000",
                },
            ),
        ]
        verifier = CloudHypervisorAlertVerifier(
            SimpleNamespace(
                ready_timeout_seconds=0,
                IDENTITY=_Identity,
            ),  # type: ignore[arg-type]
            SimpleNamespace(records=lambda: records),  # type: ignore[arg-type]
            Path("/unused"),
            SimpleNamespace(),  # type: ignore[arg-type]
        )

        with self.assertRaises(RuntimeError) as raised:
            verifier.wait_observation_alerts(
                SimpleNamespace(),  # type: ignore[arg-type]
                17,
                SimpleNamespace(),  # type: ignore[arg-type]
            )

        diagnostic = str(raised.exception)
        self.assertIn("root pid=17", diagnostic)
        self.assertIn("sandbox.process.high_read", diagnostic)
        self.assertIn("sandbox.process.high_write", diagnostic)
        self.assertIn("pid=991,start=4432", diagnostic)

    def test_provider_is_ready_before_resource_baseline_and_guest_oom(
        self,
    ) -> None:
        events: list[str] = []

        class CoordinationFile:
            def __init__(self, name: str) -> None:
                self.name = name

            def touch(self) -> None:
                events.append(f"touch:{self.name}")

        class Coordination:
            def file(self, name: str) -> CoordinationFile:
                return CoordinationFile(name)

        class Verifier:
            def wait_path(
                self,
                path: CoordinationFile,
                process: object,
                description: str,
            ) -> None:
                del process, description
                events.append(f"wait:{path.name}")

            def wait_resource_baseline(self, gateway: object) -> None:
                del gateway
                events.append("wait:resource-baseline")

            def trigger_guest_oom(self, vm: object) -> None:
                del vm
                events.append("trigger:guest-oom")

        scenario = CloudHypervisorExecutionIsolationScenario.__new__(
            CloudHypervisorExecutionIsolationScenario
        )
        scenario._context = SimpleNamespace(
            report_progress=lambda step, message: events.append(
                f"progress:{step}"
            )
        )
        scenario._verifier = Verifier()  # type: ignore[assignment]

        scenario._reach_pre_oom_checkpoint(
            object(),  # type: ignore[arg-type]
            object(),  # type: ignore[arg-type]
            object(),  # type: ignore[arg-type]
            Coordination(),  # type: ignore[arg-type]
        )

        self.assertEqual(
            events,
            [
                "progress:provider",
                "wait:provider.ready",
                "progress:resource-baseline",
                "wait:resource-baseline",
                "progress:guest-oom",
                "trigger:guest-oom",
                "touch:release",
            ],
        )

    def test_controlled_oom_runs_in_guest_root_not_workload_namespace(
        self,
    ) -> None:
        guest = _GuestCapture(
            [
                CommandResult(
                    ("guest",),
                    0,
                    f"{_Identity.OOM_KILL_MARKER} before=4 after=5\n",
                    "",
                )
            ]
        )

        class Vm:
            container_id = "owned-kata-vm"
            exec_called = False

            def exec(self, *args: object, **kwargs: object) -> CommandResult:
                del args, kwargs
                self.exec_called = True
                return CommandResult(
                    ("ctr", "tasks", "exec"),
                    70,
                    "",
                    "cgroup v2 is unavailable",
                )

        with tempfile.TemporaryDirectory() as raw:
            verifier = CloudHypervisorAlertVerifier(
                SimpleNamespace(
                    ready_timeout_seconds=9,
                    IDENTITY=_Identity,
                ),  # type: ignore[arg-type]
                SimpleNamespace(),  # type: ignore[arg-type]
                Path(raw),
                guest,  # type: ignore[arg-type]
            )
            vm = Vm()

            verifier.trigger_guest_oom(vm)  # type: ignore[arg-type]

        self.assertFalse(vm.exec_called)
        self.assertEqual(len(guest.calls), 1)
        self.assertEqual(guest.calls[0][0], vm.container_id)
        command = guest.calls[0][1]
        self.assertIn("/usr/bin/base64 -d", command)
        self.assertIn(_Identity.OOM_KILL_MARKER, command)
        self.assertNotIn("/opt/actrail-execution/oom-trigger.sh", command)

        script = (
            Path(__file__).parent / "assets" / "oom-trigger.sh"
        ).read_text(encoding="utf-8")
        self.assertIn("cgroup.controllers", script)
        self.assertIn("cgroup.procs", script)
        self.assertIn("memory.max", script)
        self.assertIn("memory.events", script)
        self.assertIn("exec awk", script)
        self.assertNotIn("python3", script)

    def test_all_three_backends_share_observer_gateway_connect_order(self) -> None:
        for backend in ("cloud-hypervisor", "stratovirt", "firecracker"):
            with self.subTest(backend=backend):
                events: list[str] = []

                class Inventory:
                    def snapshot(self) -> frozenset[Path]:
                        return frozenset()

                    def wait_new_base_socket(
                        self,
                        before: frozenset[Path],
                        timeout_seconds: float,
                    ) -> Path:
                        return Path("/run/vc/owned/vsock.sock")

                    def gateway_socket(self, base: Path, port: int) -> Path:
                        return base

                    def listener_socket(self, base: Path, port: int) -> Path:
                        return Path(f"{base}_{port}")

                    def wait_listener_socket(
                        self,
                        base: Path,
                        port: int,
                        timeout_seconds: float,
                    ) -> Path:
                        events.append("gateway.listener-ready")
                        return Path(f"{base}_{port}")

                class Vm:
                    container_id = "owned"

                    def start(self) -> None:
                        events.append("vm.start")

                class Gateway:
                    def wait_for_output(self, marker: str, *, timeout: float) -> None:
                        events.append("gateway.output-ready")

                    def terminate(self, *, grace_seconds: float) -> CommandResult:
                        return CommandResult(("gateway",), 0, "", "")

                scenario = CloudHypervisorExecutionIsolationScenario.__new__(
                    CloudHypervisorExecutionIsolationScenario
                )
                scenario._inventory = (
                    None if backend == "stratovirt" else Inventory()
                )  # type: ignore[assignment]
                scenario._config = SimpleNamespace(
                    BACKEND=backend,
                    vsock_port=43182,
                    ready_timeout_seconds=7,
                )
                scenario._observer = SimpleNamespace(
                    require_ready_and_unconnected=lambda vm: events.append(
                        "observer.ready-unconnected"
                    ),
                    connect=lambda vm: events.append("observer.connect"),
                )
                scenario._setup = SimpleNamespace(
                    write_gateway_config=lambda socket_path, daemon_port: events.append(
                        "gateway.configure"
                    )
                )
                scenario._verifier = SimpleNamespace(
                    require_alive=lambda process, name: events.append(
                        "gateway.process-alive"
                    )
                )

                def start_gateway() -> Gateway:
                    events.append("gateway.start")
                    return Gateway()

                scenario._start_gateway = (  # type: ignore[method-assign]
                    start_gateway
                )
                scenario._start_vm_gateway_and_connect(
                    Vm(),  # type: ignore[arg-type]
                    19472,
                    {},
                )

                self.assertLess(
                    events.index("observer.ready-unconnected"),
                    events.index("gateway.start"),
                )
                self.assertLess(
                    events.index("gateway.process-alive"),
                    events.index("observer.connect"),
                )
                if backend != "stratovirt":
                    self.assertLess(
                        events.index("gateway.listener-ready"),
                        events.index("observer.connect"),
                    )
                else:
                    self.assertLess(
                        events.index("gateway.output-ready"),
                        events.index("observer.connect"),
                    )

    def test_hybrid_lifecycle_waits_for_listener_before_explicit_connect(
        self,
    ) -> None:
        events: list[str] = []
        base = Path("/run/vc/firecracker/sandbox/root/kata.hvsock")
        endpoint = Path(f"{base}_43182")

        class Inventory:
            def snapshot(self) -> frozenset[Path]:
                events.append("inventory.snapshot")
                return frozenset()

            def wait_new_base_socket(
                self,
                before: frozenset[Path],
                timeout_seconds: float,
            ) -> Path:
                self.before = before
                self.timeout_seconds = timeout_seconds
                events.append("inventory.base-ready")
                return base

            def gateway_socket(self, discovered: Path, port: int) -> Path:
                self.discovered = discovered
                self.port = port
                events.append("inventory.gateway-socket")
                return base

            def listener_socket(self, discovered: Path, port: int) -> Path:
                return endpoint

            def wait_listener_socket(
                self,
                discovered: Path,
                port: int,
                timeout_seconds: float,
            ) -> Path:
                events.append("inventory.listener-ready")
                return endpoint

        class Vm:
            container_id = "fc-alert-owned"

            def start(self) -> None:
                events.append("vm.start")

        class Observer:
            def require_ready_and_unconnected(self, vm: Vm) -> None:
                events.append("observer.ready-unconnected")

            def connect(self, vm: Vm) -> None:
                events.append("observer.connect")

        class Setup:
            def write_gateway_config(
                self,
                socket_path: Path | None,
                daemon_port: int,
            ) -> None:
                self.socket_path = socket_path
                self.daemon_port = daemon_port
                events.append("gateway.configure")

        class Verifier:
            def require_alive(self, process: object, name: str) -> None:
                events.append("gateway.alive")

        scenario = CloudHypervisorExecutionIsolationScenario.__new__(
            CloudHypervisorExecutionIsolationScenario
        )
        scenario._inventory = Inventory()  # type: ignore[assignment]
        scenario._config = SimpleNamespace(
            vsock_port=43182,
            ready_timeout_seconds=7,
        )
        scenario._observer = Observer()  # type: ignore[assignment]
        scenario._setup = Setup()  # type: ignore[assignment]
        scenario._verifier = Verifier()  # type: ignore[assignment]
        gateway = object()

        def start_gateway() -> object:
            events.append("gateway.start")
            return gateway

        scenario._start_gateway = start_gateway  # type: ignore[method-assign]
        results: dict[str, object] = {}
        returned = scenario._start_vm_gateway_and_connect(
            Vm(),  # type: ignore[arg-type]
            19472,
            results,  # type: ignore[arg-type]
        )

        self.assertIs(returned, gateway)
        self.assertEqual(
            events,
            [
                "inventory.snapshot",
                "vm.start",
                "observer.ready-unconnected",
                "inventory.base-ready",
                "inventory.gateway-socket",
                "gateway.configure",
                "gateway.start",
                "gateway.alive",
                "inventory.listener-ready",
                "observer.connect",
            ],
        )
        self.assertEqual(
            set(results),
            {"observer-ready", "gateway", "observer-connect"},
        )

    def test_native_lifecycle_connects_only_after_observer_and_gateway(self) -> None:
        events: list[str] = []

        class Vm:
            container_id = "stratovirt-alert-owned"

            def start(self) -> None:
                events.append("vm.start")

        scenario = CloudHypervisorExecutionIsolationScenario.__new__(
            CloudHypervisorExecutionIsolationScenario
        )
        scenario._inventory = None
        scenario._config = SimpleNamespace(
            vsock_port=43182,
            ready_timeout_seconds=7,
        )
        scenario._observer = SimpleNamespace(
            require_ready_and_unconnected=lambda vm: events.append(
                "observer.ready-unconnected"
            ),
            connect=lambda vm: events.append("observer.connect"),
        )
        scenario._setup = SimpleNamespace(
            write_gateway_config=lambda socket_path, daemon_port: events.append(
                "gateway.configure-native"
            )
        )
        scenario._verifier = SimpleNamespace(
            require_alive=lambda process, name: events.append("gateway.alive")
        )
        class Gateway:
            def wait_for_output(
                self,
                marker: str,
                *,
                timeout: float,
            ) -> None:
                self.marker = marker
                self.timeout = timeout
                events.append("gateway.output-ready")

        gateway = Gateway()

        def start_gateway() -> object:
            events.append("gateway.start")
            return gateway

        scenario._start_gateway = start_gateway  # type: ignore[method-assign]
        scenario._start_vm_gateway_and_connect(
            Vm(),  # type: ignore[arg-type]
            19472,
            {},
        )

        self.assertEqual(
            events,
            [
                "vm.start",
                "observer.ready-unconnected",
                "gateway.configure-native",
                "gateway.start",
                "gateway.alive",
                "gateway.output-ready",
                "observer.connect",
            ],
        )
        self.assertEqual(gateway.marker, "gateway ready gateway_id=")
        self.assertEqual(gateway.timeout, 7)

    def test_gateway_is_terminated_when_explicit_connect_fails(self) -> None:
        events: list[str] = []

        class Vm:
            container_id = "fc-alert-owned"

            def start(self) -> None:
                events.append("vm.start")

        class Gateway:
            def wait_for_output(self, marker: str, *, timeout: float) -> None:
                events.append("gateway.output-ready")

            def terminate(self, *, grace_seconds: float) -> CommandResult:
                events.append(f"gateway.terminate:{grace_seconds}")
                return CommandResult(("gateway",), 0, "", "")

        def fail_connect(vm: Vm) -> None:
            events.append("observer.connect")
            raise RuntimeError("connect failed")

        scenario = CloudHypervisorExecutionIsolationScenario.__new__(
            CloudHypervisorExecutionIsolationScenario
        )
        scenario._inventory = None
        scenario._config = SimpleNamespace(
            vsock_port=43182,
            ready_timeout_seconds=7,
        )
        scenario._observer = SimpleNamespace(
            require_ready_and_unconnected=lambda vm: events.append(
                "observer.ready-unconnected"
            ),
            connect=fail_connect,
        )
        scenario._setup = SimpleNamespace(
            write_gateway_config=lambda socket_path, daemon_port: events.append(
                "gateway.configure"
            )
        )
        scenario._verifier = SimpleNamespace(
            require_alive=lambda process, name: events.append("gateway.alive")
        )
        scenario._start_gateway = Gateway  # type: ignore[method-assign]

        with self.assertRaisesRegex(RuntimeError, "connect failed"):
            scenario._start_vm_gateway_and_connect(
                Vm(),  # type: ignore[arg-type]
                19472,
                {},
            )

        self.assertEqual(events[-2:], ["observer.connect", "gateway.terminate:3"])

    def test_guest_system_observer_uses_bounded_unconnected_gate_then_connects(
        self,
    ) -> None:
        guest = _GuestCapture(
            [
                CommandResult(("guest",), 0, "", ""),
                CommandResult(
                    ("guest",),
                    0,
                    "actrail-sb connected sb_id=17 generation=1 reused=false\n",
                    "",
                ),
            ]
        )
        config = SimpleNamespace(
            ready_timeout_seconds=9,
            vsock_host_cid=2,
            vsock_port=43182,
            IDENTITY=_Identity,
        )
        observer = GuestSystemSandboxObserver(
            guest,  # type: ignore[arg-type]
            config,  # type: ignore[arg-type]
        )
        vm = SimpleNamespace(container_id="fc-alert-owned")

        observer.require_ready_and_unconnected(vm)  # type: ignore[arg-type]
        observer.connect(vm)  # type: ignore[arg-type]

        ready_command = guest.calls[0][1]
        self.assertIn("remaining=8", ready_command)
        self.assertIn("while [ $remaining -gt 0 ]", ready_command)
        self.assertIn(
            "/dev/actrail/sandbox-observer-control.sock",
            ready_command,
        )
        self.assertIn("connected=false publication_enabled=false", ready_command)
        self.assertIn(
            "/usr/lib/systemd/system/actrail-sb-connect.service",
            ready_command,
        )
        self.assertIn("! systemctl is-enabled --quiet", ready_command)
        self.assertIn("! systemctl is-active --quiet", ready_command)
        self.assertIn(
            "systemctl --no-pager --full status actrail-sb.service",
            ready_command,
        )
        self.assertIn(
            "systemctl --no-pager --full status actrail-sb-connect.service",
            ready_command,
        )
        self.assertIn("ls -la /dev/actrail", ready_command)
        self.assertIn(
            "tail -n 80 /dev/actrail/sandbox-observer.log",
            ready_command,
        )
        connect_command = guest.calls[1][1]
        self.assertIn("/usr/local/bin/actrail-sb connect", connect_command)
        self.assertIn("--host-cid 2", connect_command)
        self.assertIn("--port 43182", connect_command)
        self.assertEqual(guest.calls[0][2], 11)
        self.assertLessEqual(guest.calls[1][2], 7)

    def test_explicit_connect_retries_a_transient_native_listener_race(self) -> None:
        guest = _GuestCapture(
            [
                CommandResult(("guest",), 1, "", "connection refused"),
                CommandResult(
                    ("guest",),
                    0,
                    "actrail-sb connected sb_id=21 generation=1 reused=false\n",
                    "",
                ),
            ]
        )
        observer = GuestSystemSandboxObserver(
            guest,  # type: ignore[arg-type]
            SimpleNamespace(
                ready_timeout_seconds=9,
                vsock_host_cid=2,
                vsock_port=43182,
                IDENTITY=_Identity,
            ),  # type: ignore[arg-type]
        )

        with patch(
            "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
            "scenario.system_observer.time.sleep"
        ):
            observer.connect(  # type: ignore[arg-type]
                SimpleNamespace(container_id="stratovirt-alert-owned")
            )

        self.assertEqual(len(guest.calls), 2)
        self.assertIn("--request-timeout-ms", guest.calls[0][1])
        self.assertEqual(guest.calls[0][0], guest.calls[1][0])

    def test_firecracker_requirements_have_no_host_mounts(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            config = self._firecracker_config(root)
            deployment = SimpleNamespace(
                data_config=root / "configuration-firecracker.toml",
                workload_image=config.image,
                workload_image_archive=root / "firecracker-workload.tar",
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

        self.assertEqual(requirements.mounts, ())
        self.assertEqual(requirements.profile.snapshotter, "devmapper")
        self.assertEqual(
            requirements.image.archive,
            root / "firecracker-workload.tar",
        )
        self.assertEqual(requirements.name_prefix, "fc-alert")
        self.assertFalse(requirements.privileged_without_host_devices)

    def test_firecracker_running_validator_does_not_require_a_host_mount(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            config = self._firecracker_config(root)
            setup = CloudHypervisorScenarioSetup(
                config,
                SimpleNamespace(),  # type: ignore[arg-type]
                _RecordingRunner(),  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )

            class Vm:
                command = ""

                def is_running(self) -> bool:
                    return True

                def exec(
                    self,
                    command: tuple[str, ...],
                    **kwargs: object,
                ) -> CommandResult:
                    self.command = command[2]
                    return CommandResult(command, 0, "", "")

            vm = Vm()
            check = setup._validate_vm_ready(vm)  # type: ignore[arg-type]

        self.assertTrue(check.ready)
        self.assertIn("command -v python3", vm.command)
        self.assertNotIn("/run/actrail", vm.command)
        self.assertNotIn("xiaoo-real", vm.command)

    def test_shared_fs_running_validator_leaves_system_observer_to_guest_console(
        self,
    ) -> None:
        for backend in ("cloud-hypervisor", "stratovirt"):
            with self.subTest(backend=backend), tempfile.TemporaryDirectory() as raw:
                root = Path(raw)
                setup = CloudHypervisorScenarioSetup(
                    SimpleNamespace(
                        BACKEND=backend,
                        ready_timeout_seconds=9,
                        IDENTITY=_Identity,
                    ),  # type: ignore[arg-type]
                    SimpleNamespace(),  # type: ignore[arg-type]
                    _RecordingRunner(),  # type: ignore[arg-type]
                    root,
                    root / "assets",
                    root / "coord",
                )

                class Vm:
                    command = ""
                    timeout: object = None

                    def is_running(self) -> bool:
                        return True

                    def exec(
                        self,
                        command: tuple[str, ...],
                        **kwargs: object,
                    ) -> CommandResult:
                        self.command = command[2]
                        self.timeout = kwargs.get("timeout")
                        return CommandResult(command, 0, "", "")

                vm = Vm()
                check = setup._validate_vm_ready(vm)  # type: ignore[arg-type]

                self.assertTrue(check.ready)
                self.assertIn("xiaoo-real", vm.command)
                self.assertNotIn(
                    "/run/actrail/sandbox-observer-control.sock",
                    vm.command,
                )
                self.assertNotIn(
                    "/run/actrail/sandbox-observer.ready",
                    vm.command,
                )
                self.assertNotIn(
                    "systemctl --no-pager --full status actrail-sb.service",
                    vm.command,
                )
                self.assertNotIn("ls -la /run/actrail", vm.command)
                self.assertNotIn(
                    "cat /run/actrail/sandbox-observer.log",
                    vm.command,
                )
                self.assertEqual(vm.timeout, 11)
                self.assertNotIn("/run/actrail/control.sock", vm.command)

    def test_firecracker_assets_and_coordination_use_guest_exec(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            assets = root / "host-assets"
            assets.mkdir()
            (assets / "xiaoo-real").write_text(
                "#!/bin/sh\nprintf '%s\\n' --tools\n",
                encoding="utf-8",
            )
            (assets / "xiaoo-real").chmod(0o755)
            (assets / "xiaoo-root").write_text(
                "#!/bin/sh\nexit 0\n",
                encoding="utf-8",
            )
            (assets / "xiaoo-root").chmod(0o755)
            (assets / "workload.sh").write_text("payload\n", encoding="utf-8")
            CloudHypervisorAssetBundle.write_manifest(assets)
            guest_assets = root / "guest-assets"
            guest_assets.mkdir()
            guest_xiaoo = guest_assets / "xiaoo-real"
            guest_xiaoo.write_bytes((assets / "xiaoo-real").read_bytes())
            guest_xiaoo.chmod(0o755)
            guest_coordination = root / "guest-coordination"
            vm = _LocalVm()
            transport = FirecrackerAssetTransport(
                assets,
                os.getuid(),
                os.getgid(),
                10,
                asset_root=guest_assets,
                coordination_root=guest_coordination,
            )

            transport.stage(vm)  # type: ignore[arg-type]
            coordination = GuestCoordination(
                vm,  # type: ignore[arg-type]
                os.getuid(),
                os.getgid(),
                10,
                root=guest_coordination,
            )
            coordination.file("release").touch()

            self.assertEqual(
                (guest_assets / "workload.sh").read_text(encoding="utf-8"),
                "payload\n",
            )
            self.assertTrue((guest_coordination / "release").is_file())
            self.assertEqual(vm.calls[0][0][0], "python3")
            self.assertTrue(vm.calls[0][1]["input_text"])

    def test_firecracker_asset_transport_uses_preinstalled_xiaoo(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            assets = root / "host-assets"
            assets.mkdir()
            xiaoo_payload = b"#!/bin/sh\nprintf '%s\\n' --tools\n"
            (assets / "xiaoo-real").write_bytes(xiaoo_payload)
            (assets / "xiaoo-real").chmod(0o755)
            (assets / "xiaoo-root").write_text(
                "#!/bin/sh\nexit 0\n",
                encoding="utf-8",
            )
            (assets / "xiaoo-root").chmod(0o755)
            (assets / "workload.sh").write_text(
                "payload\n",
                encoding="utf-8",
            )
            CloudHypervisorAssetBundle.write_manifest(assets)
            guest_assets = root / "guest-assets"
            guest_assets.mkdir()
            (guest_assets / "xiaoo-real").write_bytes(xiaoo_payload)
            (guest_assets / "xiaoo-real").chmod(0o755)
            vm = _LocalVm()

            FirecrackerAssetTransport(
                assets,
                os.getuid(),
                os.getgid(),
                10,
                asset_root=guest_assets,
                coordination_root=root / "guest-coordination",
            ).stage(vm)  # type: ignore[arg-type]

            encoded = vm.calls[0][1]["input_text"]
            assert isinstance(encoded, str)
            with tarfile.open(
                fileobj=io.BytesIO(base64.b64decode(encoded)),
                mode="r:gz",
            ) as archive:
                transported_names = archive.getnames()

        self.assertNotIn("xiaoo-real", transported_names)
        self.assertIn("workload.sh", transported_names)

    def test_firecracker_asset_transport_rejects_wrong_preinstalled_xiaoo(
        self,
    ) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            assets = root / "host-assets"
            assets.mkdir()
            (assets / "xiaoo-real").write_bytes(b"expected-xiaoo\n")
            (assets / "xiaoo-real").chmod(0o755)
            (assets / "xiaoo-root").write_text(
                "#!/bin/sh\nexit 0\n",
                encoding="utf-8",
            )
            (assets / "xiaoo-root").chmod(0o755)
            (assets / "workload.sh").write_text(
                "payload\n",
                encoding="utf-8",
            )
            CloudHypervisorAssetBundle.write_manifest(assets)
            guest_assets = root / "guest-assets"
            guest_assets.mkdir()
            (guest_assets / "xiaoo-real").write_bytes(b"wrong-xiaoo\n")
            (guest_assets / "xiaoo-real").chmod(0o755)

            with self.assertRaisesRegex(RuntimeError, "failed verification"):
                FirecrackerAssetTransport(
                    assets,
                    os.getuid(),
                    os.getgid(),
                    10,
                    asset_root=guest_assets,
                    coordination_root=root / "guest-coordination",
                ).stage(_LocalVm())  # type: ignore[arg-type]

    def test_firecracker_asset_stage_does_not_wait_for_stdin_eof(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            assets = root / "host-assets"
            assets.mkdir()
            (assets / "xiaoo-real").write_bytes(b"preinstalled-xiaoo")
            (assets / "xiaoo-root").write_text("root\n", encoding="utf-8")
            (assets / "workload.sh").write_text(
                "payload\n",
                encoding="utf-8",
            )
            CloudHypervisorAssetBundle.write_manifest(assets)

            class CaptureVm:
                calls: list[tuple[tuple[str, ...], dict[str, object]]] = []

                def exec(
                    self,
                    command: tuple[str, ...],
                    **kwargs: object,
                ) -> CommandResult:
                    self.calls.append((command, kwargs))
                    return CommandResult(command, 0, "", "")

            vm = CaptureVm()
            FirecrackerAssetTransport(
                assets,
                os.getuid(),
                os.getgid(),
                10,
                asset_root=root / "guest-assets",
                coordination_root=root / "guest-coordination",
            ).stage(vm)  # type: ignore[arg-type]
            command, options = vm.calls[0]
            input_text = options["input_text"]
            assert isinstance(input_text, str)
            process = subprocess.Popen(
                command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
            )
            assert process.stdin is not None
            process.stdin.write(input_text)
            process.stdin.flush()
            timed_out = False
            try:
                returncode = process.wait(timeout=3)
            except subprocess.TimeoutExpired:
                timed_out = True
                process.kill()
                returncode = process.wait(timeout=3)
            finally:
                process.stdin.close()
            assert process.stdout is not None
            assert process.stderr is not None
            stdout = process.stdout.read()
            stderr = process.stderr.read()
            process.stdout.close()
            process.stderr.close()

        self.assertFalse(
            timed_out,
            "Guest asset staging waited for ctr to propagate stdin EOF",
        )
        self.assertEqual(returncode, 0, stderr)
        self.assertIn("ACTRAIL_FIRECRACKER_ASSETS_STAGED", stdout)

    def test_firecracker_asset_staging_uses_workload_runtime_timeout(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            config = self._firecracker_config(root)
            setup = CloudHypervisorScenarioSetup(
                config,
                SimpleNamespace(),  # type: ignore[arg-type]
                _RecordingRunner(),  # type: ignore[arg-type]
                root,
                root / "assets",
                root / "coord",
            )
            vm = object()
            with patch(
                "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
                "scenario.setup.FirecrackerAssetTransport"
            ) as transport:
                setup.stage_assets(vm)  # type: ignore[arg-type]

        transport.assert_called_once_with(
            root / "assets",
            config.workload_uid,
            config.workload_gid,
            config.runtime_timeout_seconds,
        )
        transport.return_value.stage.assert_called_once_with(vm)

    def test_hybrid_socket_inventories_use_backend_specific_base_paths(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            root = Path(raw)
            cloud_base = root / "cloud-owned" / "clh.sock"
            firecracker_base = root / "fc-owned" / "root" / "kata.hvsock"
            cloud_base.parent.mkdir(parents=True)
            firecracker_base.parent.mkdir(parents=True)
            cloud_socket = socket.socket(socket.AF_UNIX)
            firecracker_socket = socket.socket(socket.AF_UNIX)
            cloud_listener = socket.socket(socket.AF_UNIX)
            firecracker_listener = socket.socket(socket.AF_UNIX)
            try:
                cloud_socket.bind(str(cloud_base))
                firecracker_socket.bind(str(firecracker_base))
                cloud = CloudHypervisorSocketInventory(root)
                firecracker = FirecrackerSocketInventory(root)
                discovered_cloud = next(iter(cloud.snapshot()))
                discovered_firecracker = next(iter(firecracker.snapshot()))

                self.assertEqual(
                    cloud.gateway_socket(discovered_cloud, 43182),
                    Path(f"{discovered_cloud}_43182"),
                )
                self.assertEqual(
                    firecracker.gateway_socket(discovered_firecracker, 43182),
                    discovered_firecracker,
                )
                cloud_listener.bind(f"{discovered_cloud}_43182")
                firecracker_listener.bind(f"{discovered_firecracker}_43182")
                self.assertEqual(
                    cloud.wait_listener_socket(discovered_cloud, 43182, 1),
                    Path(f"{discovered_cloud}_43182"),
                )
                self.assertEqual(
                    firecracker.wait_listener_socket(
                        discovered_firecracker,
                        43182,
                        1,
                    ),
                    Path(f"{discovered_firecracker}_43182"),
                )
            finally:
                firecracker_listener.close()
                cloud_listener.close()
                firecracker_socket.close()
                cloud_socket.close()

    def test_firecracker_devmapper_probe_requires_ok_status(self) -> None:
        okay = CommandResult(
            ("ctr",),
            0,
            "TYPE ID PLATFORMS STATUS\n"
            "io.containerd.snapshotter.v1 devmapper linux/amd64 ok\n",
            "",
        )
        error = CommandResult(
            ("ctr",),
            0,
            "TYPE ID PLATFORMS STATUS\n"
            "io.containerd.snapshotter.v1 devmapper linux/amd64 error\n",
            "",
        )
        config = SimpleNamespace(
            command_timeout_seconds=11,
        )
        with patch(
            "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
            "prerequisites.shutil.which",
            return_value="/sbin/dmsetup",
        ):
            okay_runner = _RecordingRunner([okay])
            okay_problem = CloudHypervisorExecutionIsolationPrerequisites(
                config,  # type: ignore[arg-type]
                okay_runner,
            )._firecracker_devmapper_problem()
            error_problem = CloudHypervisorExecutionIsolationPrerequisites(
                config,  # type: ignore[arg-type]
                _RecordingRunner([error]),
            )._firecracker_devmapper_problem()
        missing_runner = _RecordingRunner()
        with patch(
            "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
            "prerequisites.shutil.which",
            return_value=None,
        ):
            missing_problem = CloudHypervisorExecutionIsolationPrerequisites(
                config,  # type: ignore[arg-type]
                missing_runner,
            )._firecracker_devmapper_problem()

        self.assertIsNone(okay_problem)
        self.assertIn("status=error", error_problem or "")
        self.assertIn("dmsetup is unavailable", missing_problem or "")
        self.assertEqual(missing_runner.calls, [])
        self.assertEqual(
            okay_runner.calls[0][0],
            (
                "ctr",
                "plugins",
                "list",
                "type==io.containerd.snapshotter.v1,id==devmapper",
            ),
        )
        self.assertEqual(okay_runner.calls[0][1]["timeout"], 11)

    def test_firecracker_unavailable_devmapper_is_external_skip(self) -> None:
        with tempfile.TemporaryDirectory() as raw:
            config = self._firecracker_config(Path(raw))
            runner = _RecordingRunner(
                [
                    CommandResult(
                        ("ctr",),
                        0,
                        "TYPE ID PLATFORMS STATUS\n"
                        "io.containerd.snapshotter.v1 devmapper linux/amd64 skip\n",
                        "",
                    )
                ]
            )
            with (
                patch(
                    "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
                    "prerequisites.platform.machine",
                    return_value="x86_64",
                ),
                patch(
                    "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
                    "prerequisites.os.access",
                    return_value=True,
                ),
                patch(
                    "tests.v2.regression.execution_isolation_cloud_hypervisor.v2."
                    "prerequisites.shutil.which",
                    return_value="/usr/bin/tool",
                ),
            ):
                problem = CloudHypervisorExecutionIsolationPrerequisites(
                    config,
                    runner,
                )._external_problem()

        self.assertIsNotNone(problem)
        assert problem is not None
        self.assertIs(problem.status, TestStatus.SKIPPED)
        self.assertIn("devmapper snapshotter is unavailable", problem.message)

    @staticmethod
    def _firecracker_config(root: Path) -> FirecrackerExecutionIsolationConfig:
        inputs = TestCaseInputs(root, root, root)
        with patch.dict(os.environ, {}, clear=True):
            return FirecrackerExecutionIsolationConfig.from_environment(inputs)


if __name__ == "__main__":
    unittest.main()
