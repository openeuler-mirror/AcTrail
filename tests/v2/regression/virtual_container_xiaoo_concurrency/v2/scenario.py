from __future__ import annotations

import hashlib
import os
import secrets
import shutil
import time
from pathlib import Path

from tests.v2.common.kata_runtime import (
    CtrCapabilities,
    DeploymentArtifacts,
    GuestConsole,
    KataContainerRequirements,
    KataMount,
    KataRequirementsBuilder,
    KataTestContainer,
    RequirementCheck,
)
from tests.v2.common.kata_runtime.process import ManagedProcess, SubprocessRunner
from tests.v2.common.test_case import TestResult, TestStatus
from tests.v2.common.testing_context import TestingContextSingleton
from tests.v2.regression.virtual_container.v2.assertions import (
    find_clean_trace,
    parse_summary_counts,
    reject_markers,
    require_markers,
)

from .config import VirtualContainerXiaooConcurrencyConfig


DUAL_VM_INSTANCES = ("a", "b")


class DualKataXiaooScenario:
    def __init__(
        self,
        config: VirtualContainerXiaooConcurrencyConfig,
        context: TestingContextSingleton,
        deployment: DeploymentArtifacts | None = None,
    ) -> None:
        self._config = config
        self._context = context
        self._runner = SubprocessRunner()
        self._guest = GuestConsole(self._runner)
        self._case_dir = Path(__file__).resolve().parent
        self._provider_source = (
            config.repo / "tests/support/llm-http-proxy/provider_proxy.py"
        )
        self._runtime_config = (
            deployment.data_config
            if deployment is not None
            else config.runtime_config
        )
        self._workload_bundle = (
            deployment.workload_bundle
            if deployment is not None
            else config.workload_bundle
        )
        self._xiaoo_binary = (
            deployment.xiaoo if deployment is not None else config.xiaoo_binary
        )
        self._image = (
            deployment.workload_image if deployment is not None else config.image
        )
        self._run_token = secrets.token_hex(8)
        self._run_dir = config.work_dir / f"run-{self._run_token}"
        self._assets = self._run_dir / "assets"
        self._coord_root = self._run_dir / "coord"

    def run(self) -> TestResult:
        results: dict[str, TestResult] = {}
        containers: dict[str, KataTestContainer] = {}
        processes: dict[str, ManagedProcess] = {}
        outputs: dict[str, str] = {}
        cleanup_errors: list[str] = []
        try:
            self._context.report_progress(
                "assets",
                "preparing deterministic local xiaoO/provider fixtures",
            )
            self._prepare_assets()
            capabilities = CtrCapabilities.detect(self._runner)

            self._context.report_progress(
                "vms",
                "starting two independent Kata VMs",
            )
            for instance in DUAL_VM_INSTANCES:
                container = KataTestContainer(
                    self._requirements(instance),
                    self._runner,
                    capabilities,
                )
                container.start()
                containers[instance] = container
            if not all(container.is_running() for container in containers.values()):
                raise RuntimeError("both Kata VMs did not remain Running")
            results["dual_vm_ready"] = TestResult(
                TestStatus.PASSED,
                "two independent Kata VMs are Running",
            )

            for instance in DUAL_VM_INSTANCES:
                processes[instance] = containers[instance].start_exec(
                    (
                        "/bin/sh",
                        "/opt/actrail/bin/actrail-init",
                        "--name",
                        self._trace_title(instance),
                        "--",
                        "/bin/sh",
                        "/opt/actrail-xiaoo/workload.sh",
                    ),
                    uid=self._config.workload_uid,
                    gid=self._config.workload_gid,
                    environment=self._workload_environment(instance),
                )

            self._context.report_progress(
                "providers",
                "waiting for both in-VM providers",
            )
            self._wait_for_paths(
                tuple(
                    self._coord_root / instance / "provider.ready"
                    for instance in DUAL_VM_INSTANCES
                ),
                processes,
                self._config.ready_timeout_seconds,
                "both providers to become ready",
            )
            results["provider_ready"] = TestResult(
                TestStatus.COMPOSITE,
                "both local providers became ready",
                {
                    instance: TestResult(TestStatus.PASSED, "provider.ready")
                    for instance in DUAL_VM_INSTANCES
                },
            )

            for instance in DUAL_VM_INSTANCES:
                (self._coord_root / instance / "release").touch()
            self._context.report_progress(
                "overlap",
                "proving both xiaoO processes are simultaneously active",
            )
            self._wait_for_paths(
                tuple(
                    self._coord_root / instance / "xiaoo.active"
                    for instance in DUAL_VM_INSTANCES
                ),
                processes,
                self._config.overlap_timeout_seconds,
                "both xiaoO active markers in the same polling window",
            )
            results["agent_overlap"] = TestResult(
                TestStatus.PASSED,
                "both xiaoO processes overlapped",
            )

            for instance in DUAL_VM_INSTANCES:
                process_result = processes[instance].wait(
                    timeout=self._config.runtime_timeout_seconds,
                    terminate_grace_seconds=5,
                )
                output = process_result.stdout + process_result.stderr
                outputs[instance] = output
                (self._run_dir / f"workload-{instance}.log").write_text(
                    output,
                    encoding="utf-8",
                )
                if process_result.returncode != 0:
                    raise RuntimeError(
                        f"xiaoO workload {instance} exited with "
                        f"{process_result.returncode}: {process_result.diagnostic}"
                    )
                self._validate_workload_output(instance, output)
                results[f"workload.{instance}"] = TestResult(
                    TestStatus.PASSED,
                    "provider and xiaoO exited cleanly",
                )

            self._validate_cross_instance_isolation(outputs)
            results["cross_trace_isolation"] = TestResult(
                TestStatus.PASSED,
                "instance markers remained isolated",
            )

            self._context.report_progress(
                "traces",
                "reading each trace from its own guest-root viewer",
            )
            for instance in DUAL_VM_INSTANCES:
                self._validate_trace(containers[instance], instance)
                results[f"trace.{instance}"] = TestResult(
                    TestStatus.PASSED,
                    "Completed/Clean with eBPF and network evidence",
                )

            containers["a"].close()
            if not containers["b"].is_running():
                raise RuntimeError("removing VM A stopped or corrupted VM B")
            results["lifecycle_isolation"] = TestResult(
                TestStatus.PASSED,
                "VM B remained Running after VM A was removed",
            )
        except Exception as error:
            diagnostics = [str(error)]
            for instance, container in containers.items():
                diagnostics.append(
                    f"VM {instance} host diagnostics:\n{container.diagnostics()}"
                )
                try:
                    guest = self._guest.capture(
                        container.container_id,
                        "systemctl --no-pager --full status actraild.service "
                        "|| true; journalctl --no-pager -u actraild.service "
                        "-n 100 || true; ls -ld /sys/kernel/btf/vmlinux "
                        "/sys/kernel/tracing /sys/kernel/debug/tracing "
                        "/sys/fs/bpf /dev/actrail /run/actrail 2>&1 || true",
                        timeout=self._config.ready_timeout_seconds,
                    )
                except Exception as diagnostic_error:
                    diagnostics.append(
                        f"VM {instance} guest diagnostics failed: "
                        f"{diagnostic_error}"
                    )
                else:
                    diagnostics.append(
                        f"VM {instance} guest diagnostics:\n{guest.stdout.strip()}"
                    )
            diagnostic = "\n".join(diagnostics)
            self._context.output.line(diagnostic)
            results["failure"] = TestResult(TestStatus.FAILED, diagnostic)
        finally:
            for instance in reversed(DUAL_VM_INSTANCES):
                container = containers.get(instance)
                if container is None:
                    continue
                try:
                    container.close()
                except Exception as error:
                    cleanup_errors.append(f"{instance}: {error}")
            results["cleanup"] = TestResult(
                TestStatus.FAILED if cleanup_errors else TestStatus.PASSED,
                "; ".join(cleanup_errors) if cleanup_errors else "owned VMs removed",
            )
        return TestResult(
            TestStatus.COMPOSITE,
            "two xiaoO workloads ran concurrently in independent Kata VMs",
            results,
        )

    def _prepare_assets(self) -> None:
        assert self._xiaoo_binary is not None
        self._assets.mkdir(parents=True, exist_ok=False)
        self._coord_root.mkdir(parents=True, exist_ok=False)
        shutil.copy2(self._xiaoo_binary, self._assets / "xiaoo")
        shutil.copy2(self._provider_source, self._assets / "provider_proxy.py")
        shutil.copy2(self._case_dir / "workload.sh", self._assets / "workload.sh")
        for executable in ("xiaoo", "workload.sh"):
            (self._assets / executable).chmod(0o755)
        (self._assets / "provider_proxy.py").chmod(0o644)
        for instance in DUAL_VM_INSTANCES:
            (self._assets / f"xiaoo-{instance}.toml").write_text(
                self._xiaoo_config(),
                encoding="utf-8",
            )
            (self._assets / f"task-{instance}.txt").write_text(
                f"Virtual-container task {instance.upper()}\n"
                f"Validate xiaoO in independent Kata VM {instance.upper()}.\n",
                encoding="utf-8",
            )
            coord = self._coord_root / instance
            coord.mkdir(mode=0o770)
            os.chown(coord, self._config.workload_uid, self._config.workload_gid)
        self._write_manifest(self._assets)

    def _requirements(self, instance: str) -> KataContainerRequirements:
        assert self._runtime_config is not None
        builder = KataRequirementsBuilder(
            backend=self._config.backend,
            runtime=self._config.ctr_runtime,
            runtime_config=self._runtime_config,
            image=self._image,
            runner=self._runner,
            pull_policy=self._config.image_pull_policy,
            image_archive=self._config.image_archive,
            runtime_timeout_seconds=self._config.runtime_timeout_seconds,
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            ready_timeout_seconds=self._config.ready_timeout_seconds,
        )
        return builder.build(
            name_prefix=f"xiaoo-{self._config.backend}-{instance}",
            command=("/bin/sh", "-c", "sleep 600"),
            mounts=(
                KataMount(
                    self._workload_bundle,
                    "/opt/actrail",
                    read_only=True,
                ),
                KataMount(
                    self._assets,
                    "/opt/actrail-xiaoo",
                    read_only=True,
                ),
                KataMount(
                    self._coord_root / instance,
                    "/run/actrail-xiaoo",
                    read_only=False,
                ),
                KataMount(Path("/dev/actrail"), "/run/actrail", read_only=True),
            ),
            artifact_directories=(self._workload_bundle, self._assets),
            labels=(("io.actrail.test.instance", instance),),
            running_validator=lambda vm: self._validate_vm_ready(vm, instance),
        )

    def _validate_vm_ready(
        self,
        vm: KataTestContainer,
        instance: str,
    ) -> RequirementCheck:
        if not vm.is_running():
            return RequirementCheck.not_ready(
                f"Kata VM {instance} task exited before readiness",
                refreshable=True,
            )
        result = vm.exec(
            (
                "/bin/sh",
                "-ec",
                ". /etc/os-release; [ \"${ID:-}\" = openEuler ]; "
                "command -v python3 >/dev/null; "
                "command -v base64 >/dev/null; "
                "/opt/actrail-xiaoo/xiaoo --help >/dev/null; "
                "python3 /opt/actrail-xiaoo/provider_proxy.py --help >/dev/null; "
                "remaining=180; while [ $remaining -gt 0 ]; do "
                "[ -S /run/actrail/control.sock ] && exit 0; "
                "remaining=$((remaining - 1)); sleep 0.5; done; exit 72",
            ),
            timeout=self._config.ready_timeout_seconds,
        )
        if result.returncode == 0:
            return RequirementCheck.ready_check()
        diagnostic = result.diagnostic or f"Kata VM {instance} readiness failed"
        return RequirementCheck.not_ready(
            diagnostic,
            refreshable=not any(
                marker in diagnostic.lower()
                for marker in ("permission denied", "operation not permitted", "kvm")
            ),
        )

    def _workload_environment(self, instance: str) -> dict[str, str]:
        upper = instance.upper()
        return {
            "ACTRAIL_XIAOO_INSTANCE": instance,
            "ACTRAIL_XIAOO_BIN": "/opt/actrail-xiaoo/xiaoo",
            "ACTRAIL_XIAOO_CONFIG": f"/opt/actrail-xiaoo/xiaoo-{instance}.toml",
            "ACTRAIL_XIAOO_PROMPT": (
                f"Reply with exactly ACTRAIL_KATA_XIAOO_{upper}_RESPONSE_OK "
                "and nothing else."
            ),
            "ACTRAIL_XIAOO_RESPONSE_MARKER": (
                f"ACTRAIL_KATA_XIAOO_{upper}_RESPONSE_OK"
            ),
            "ACTRAIL_XIAOO_WRITE_MARKER": (
                f"ACTRAIL_KATA_XIAOO_{upper}_FILE_WRITE_OK"
            ),
            "ACTRAIL_XIAOO_TASK_INPUT": (
                f"/opt/actrail-xiaoo/task-{instance}.txt"
            ),
            "ACTRAIL_XIAOO_COORD_DIR": "/run/actrail-xiaoo",
            "ACTRAIL_XIAOO_PROVIDER_SCRIPT": (
                "/opt/actrail-xiaoo/provider_proxy.py"
            ),
            "ACTRAIL_XIAOO_READY_TIMEOUT_SECONDS": str(
                self._config.ready_timeout_seconds
            ),
            "ACTRAIL_XIAOO_PROVIDER_DELAY_SECONDS": "1.0",
        }

    def _validate_workload_output(self, instance: str, output: str) -> None:
        upper = instance.upper()
        require_markers(
            output,
            (
                f"KATA_XIAOO_PROVIDER_READY instance={instance}",
                f"ACTRAIL_KATA_XIAOO_{upper}_RESPONSE_OK",
                f"KATA_XIAOO_WORKLOAD_OK instance={instance}",
                "deployment_permissions_selected="
                "host_ebpf:enabled,seccomp_notify:disabled",
                "deployment_permissions_degraded=false",
            ),
            context=f"xiaoO workload {instance}",
        )
        task_output = self._coord_root / instance / "task-output.txt"
        require_markers(
            task_output.read_text(encoding="utf-8"),
            (f"ACTRAIL_KATA_XIAOO_{upper}_FILE_WRITE_OK",),
            context=f"xiaoO task output {instance}",
        )

    def _validate_cross_instance_isolation(self, outputs: dict[str, str]) -> None:
        reject_markers(
            outputs["a"],
            (
                "ACTRAIL_KATA_XIAOO_B_RESPONSE_OK",
                "ACTRAIL_KATA_XIAOO_B_FILE_WRITE_OK",
            ),
            context="VM A",
        )
        reject_markers(
            outputs["b"],
            (
                "ACTRAIL_KATA_XIAOO_A_RESPONSE_OK",
                "ACTRAIL_KATA_XIAOO_A_FILE_WRITE_OK",
            ),
            context="VM B",
        )

    def _validate_trace(self, vm: KataTestContainer, instance: str) -> None:
        title_prefix = self._trace_title(instance)
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        trace_id = None
        last_error = "trace not listed"
        while time.monotonic() < deadline:
            traces = self._guest.capture(
                vm.container_id,
                "/usr/local/bin/actrailviewer --config "
                "/etc/actrail/operator.conf --output-format json traces",
                timeout=self._config.runtime_timeout_seconds,
            )
            if traces.returncode == 0:
                try:
                    trace_id = find_clean_trace(traces.stdout, title_prefix).trace_id
                    break
                except RuntimeError as error:
                    last_error = str(error)
            else:
                last_error = traces.diagnostic
            time.sleep(0.5)
        if trace_id is None:
            raise RuntimeError(
                f"timed out waiting for VM {instance} trace: {last_error}"
            )
        summary = self._guest.capture(
            vm.container_id,
            "/usr/local/bin/actrailviewer --config /etc/actrail/operator.conf "
            f"summary --trace-id {trace_id}",
            timeout=self._config.runtime_timeout_seconds,
        )
        if summary.returncode != 0:
            raise RuntimeError(
                f"VM {instance} viewer summary failed: {summary.diagnostic}"
            )
        counts = parse_summary_counts(summary.stdout)
        if counts.events <= 0 or counts.network_events <= 0:
            raise RuntimeError(
                f"VM {instance} trace lacks eBPF/network evidence: "
                f"events={counts.events} network_events={counts.network_events}"
            )

    def _wait_for_paths(
        self,
        paths: tuple[Path, ...],
        processes: dict[str, ManagedProcess],
        timeout_seconds: int,
        description: str,
    ) -> None:
        deadline = time.monotonic() + timeout_seconds
        while time.monotonic() < deadline:
            if all(path.is_file() for path in paths):
                return
            for instance, process in processes.items():
                if process.poll() is not None:
                    result = process.wait(timeout=1)
                    raise RuntimeError(
                        f"xiaoO {instance} exited before {description}: "
                        + result.diagnostic
                    )
            time.sleep(0.05)
        raise RuntimeError(f"timed out waiting for {description}")

    @staticmethod
    def _xiaoo_config() -> str:
        return """[llm]
provider = "deepseek"
model = "deepseek-chat"
api_key_env = "ACTRAIL_VIRTUAL_XIAOO_API_KEY"
api_base = "http://127.0.0.1:18098"
max_tokens = 128
context_window = 32768
reasoning_effort = "off"
"""

    @staticmethod
    def _write_manifest(directory: Path) -> None:
        lines = []
        for path in sorted(directory.iterdir()):
            if path.name == "MANIFEST.sha256" or not path.is_file():
                continue
            digest = hashlib.sha256(path.read_bytes()).hexdigest()
            lines.append(f"{digest}  ./{path.name}\n")
        (directory / "MANIFEST.sha256").write_text(
            "".join(lines),
            encoding="utf-8",
        )

    def _trace_title(self, instance: str) -> str:
        return f"virtual-container-xiaoo-{instance}-{self._run_token}"
