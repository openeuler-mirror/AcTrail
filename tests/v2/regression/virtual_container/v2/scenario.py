from __future__ import annotations

import os
import secrets
import shlex
import time
from collections.abc import Iterator
from contextlib import contextmanager
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
from tests.v2.common.kata_runtime.process import (
    CommandResult,
    CommandTimeoutError,
    SubprocessRunner,
)
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.runner import TestingContextSingleton

from .assertions import find_clean_trace, parse_summary_counts, require_markers
from .collector_evidence import wait_for_spans
from .config import VirtualContainerConfig
from .matrix import DATA_CASES, INTERFACE_CASES, TRACE_TITLE_TOKEN, DataCase
from .prerequisites import ResolvedBackend

from .constants import CASE_CONSTANTS


class VirtualContainerScenario:
    def __init__(
        self,
        config: VirtualContainerConfig,
        context: TestingContextSingleton,
        deployment: DeploymentArtifacts | None = None,
    ) -> None:
        self._deployment = deployment
        self._config = config
        self._context = context
        self._runner = SubprocessRunner()
        self._guest = GuestConsole(self._runner)
        self._case_dir = Path(__file__).resolve().parents[1]
        self._tmp_dir = config.work_dir / "tmp"
        self._guest_bundle = (
            deployment.guest_bundle
            if deployment is not None
            else config.capability_bundle
        )
        self._workload_bundle = (
            deployment.workload_bundle
            if deployment is not None
            else config.workload_bundle
        )
        self._capabilities: CtrCapabilities | None = None

    def run_contracts(self) -> TestResult:
        self._tmp_dir.mkdir(parents=True, exist_ok=True)
        environment = self._base_environment()
        results: dict[str, TestResult] = {}
        contract_bundle = self._tmp_dir / "contract-guest-bundle"
        self._context.report_progress(
            "contracts",
            "preparing the shared guest bundle fixture",
        )
        results["guest_bundle_fixture"] = self._run_command(
            [str(self._case_dir / "prepare-guest-bundle.sh")],
            environment=environment
            | {
                "ACTRAIL_BUILD": "0",
                "BUNDLE_DIR": str(contract_bundle),
            },
            timeout=self._config.command_timeout_seconds,
        )
        if results["guest_bundle_fixture"].has_failure():
            return TestResult(
                TestStatus.COMPOSITE,
                "virtual-container static and fake-runtime contracts",
                results,
            )
        environment |= {"BUNDLE_DIR": str(contract_bundle)}
        static_contracts = (
            ("guest_systemd", "test-guest-systemd-contract.sh"),
            ("guest_otel_endpoint", "test-guest-otel-endpoint-contract.sh"),
            ("host_collector", "test-host-collector-contract.sh"),
            ("vsock_egress", "test-vsock-egress-contract.sh"),
            (
                "openeuler_vsock_deploy",
                "test-openeuler-vsock-deploy-contract.sh",
            ),
            ("guest_rootfs_install", "test-install-guest-rootfs.sh"),
            ("workload_interface", "test-workload-interface-contract.sh"),
            ("workload_bundle", "test-prepare-workload-bundle.sh"),
            ("pid_namespace_assertion", "test-assert-pid-namespace.sh"),
            ("preflight_backend", "test-preflight-backend-contract.sh"),
            ("preflight_cgroup", "test-preflight-cgroup-contract.sh"),
            ("local_test_profile", "test-local-profile-contract.py"),
            ("v2_runner_entrypoint", "test-run-v2-tests-entrypoint.sh"),
            ("runtime_config_paths", "test-runtime-config-paths.sh"),
            ("kata_3_32_host", "test-kata-3.32-host-contract.sh"),
            ("openeuler_image_builder", "test-openeuler-image-builder-contract.sh"),
            (
                "openeuler_kata_kernel_builder",
                "test-openeuler-kata-kernel-builder-contract.sh",
            ),
            ("guest_bundle_rebuild", "test-prepare-guest-bundle.sh"),
        )
        for name, script in static_contracts:
            self._context.report_progress("contracts", name)
            results[name] = self._run_command(
                [str(self._case_dir / script)],
                environment=environment,
                timeout=self._config.command_timeout_seconds,
            )

        results["kata_runtime_manager"] = self._run_command(
            [
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "tests/v2/common/kata_runtime",
                "-p",
                "test_*.py",
                "-q",
            ],
            environment=environment,
            timeout=self._config.command_timeout_seconds,
        )
        results["virtual_container_v2_modules"] = self._run_command(
            [
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "tests/v2/regression/virtual_container/v2",
                "-p",
                "test_*.py",
                "-q",
            ],
            environment=environment,
            timeout=self._config.command_timeout_seconds,
        )
        results["virtual_container_xiaoo_v2_modules"] = self._run_command(
            [
                "python3",
                "-m",
                "unittest",
                "discover",
                "-s",
                "tests/v2/regression/virtual_container_xiaoo_concurrency/v2",
                "-p",
                "test_*.py",
                "-q",
            ],
            environment=environment,
            timeout=self._config.command_timeout_seconds,
        )
        return TestResult(
            TestStatus.COMPOSITE,
            "virtual-container static and fake-runtime contracts",
            results,
        )

    def run_backend(self, backend: ResolvedBackend) -> TestResult:
        assert backend.problem is None
        assert backend.runtime_config is not None
        assert backend.data_config is not None
        results: dict[str, TestResult] = {}
        progress_backend = _progress_backend(backend.name)
        self._context.report_progress(
            f"{progress_backend}.preflight",
            "validating host runtime and deployment paths",
        )
        results["preflight"] = self._run_command(
            [str(self._case_dir / "preflight.sh")],
            environment=self._base_environment()
            | {
                "BACKEND": backend.name,
                "CTR_RUNTIME": self._config.ctr_runtime,
                "RUNTIME_CONFIG_PATH": str(backend.runtime_config),
            },
            timeout=self._config.command_timeout_seconds,
        )
        if results["preflight"].has_failure():
            return TestResult(
                TestStatus.COMPOSITE,
                f"{backend.name} virtual-container acceptance",
                results,
            )

        if self._capabilities is None:
            self._capabilities = CtrCapabilities.detect(self._runner)
        self._context.report_progress(
            f"{progress_backend}.base",
            "starting one reusable interface VM",
        )
        try:
            with self._network_namespace() as network_namespace:
                results["interface"] = self._run_interface_matrix(
                    backend,
                    network_namespace,
                )
        except Exception as error:
            results["interface"] = TestResult(TestStatus.FAILED, str(error))

        results["data_config"] = self._run_command(
            [
                str(self._case_dir / "validate-runtime-config.py"),
                "--backend",
                backend.name,
                "--require-kernel-config",
                "--require-ebpf",
                str(backend.data_config),
            ],
            environment=self._base_environment(),
            timeout=self._config.command_timeout_seconds,
        )
        if results["data_config"].has_failure():
            return TestResult(
                TestStatus.COMPOSITE,
                f"{backend.name} virtual-container acceptance",
                results,
            )
        self._context.report_progress(
            f"{progress_backend}.data",
            "starting one reusable guest-root data VM",
        )
        try:
            with self._network_namespace() as network_namespace:
                results["data"] = self._run_data_matrix(
                    backend,
                    network_namespace,
                )
        except Exception as error:
            results["data"] = TestResult(TestStatus.FAILED, str(error))
        return TestResult(
            TestStatus.COMPOSITE,
            f"{backend.name} virtual-container acceptance",
            results,
        )

    def _run_interface_matrix(
        self,
        backend: ResolvedBackend,
        network_namespace: Path | None = None,
    ) -> TestResult:
        assert backend.runtime_config is not None
        requirements = self._requirements(
            backend.name,
            backend.runtime_config,
            profile_name="base",
            data=False,
            network_namespace=network_namespace,
        )
        results: dict[str, TestResult] = {}
        assert self._capabilities is not None
        with KataTestContainer(
            requirements,
            self._runner,
            self._capabilities,
        ) as vm:
            for case in INTERFACE_CASES:
                self._context.report_progress(
                    f"{_progress_backend(backend.name)}.if.{case.name}",
                    "running in reusable base VM",
                )
                title = self._trace_title(backend.name, f"interface-{case.name}")
                command = tuple(
                    title if value == TRACE_TITLE_TOKEN else value
                    for value in case.command
                )
                run_gid = self._config.workload_gid + case.gid_delta
                if run_gid > 2147483647:
                    run_gid = self._config.workload_gid - 1
                environment = (
                    {"ACTRAIL_NAMESPACE_TRACE_NAME": title}
                    if case.name == "namespace"
                    else None
                )
                result = vm.exec(
                    command,
                    uid=self._config.workload_uid,
                    gid=run_gid,
                    environment=environment,
                    timeout=self._config.runtime_timeout_seconds,
                )
                output = result.stdout + result.stderr
                (self._tmp_dir / f"{backend.name}-interface-{case.name}.txt").write_text(output, encoding="utf-8")
                try:
                    if case.expect_failure:
                        if result.returncode == 0:
                            raise RuntimeError(
                                "wrong-GID workload unexpectedly reached AcTrail"
                            )
                    elif result.returncode != 0:
                        raise RuntimeError(
                            f"workload exited with {result.returncode}: "
                            f"{result.diagnostic}"
                        )
                    require_markers(output, case.expected_markers, context=case.name)
                except RuntimeError as error:
                    results[case.name] = TestResult(
                        TestStatus.FAILED,
                        self._failure_diagnostics(vm, error),
                    )
                else:
                    results[case.name] = TestResult(TestStatus.PASSED, "completed")
        return TestResult(
            TestStatus.COMPOSITE,
            "four interface cells in one Kata VM",
            results,
        )

    def _run_data_matrix(
        self,
        backend: ResolvedBackend,
        network_namespace: Path | None = None,
    ) -> TestResult:
        assert backend.data_config is not None
        requirements = self._requirements(
            backend.name,
            backend.data_config,
            profile_name="data",
            data=True,
            network_namespace=network_namespace,
        )
        started_unix_nano = time.time_ns()
        results: dict[str, TestResult] = {}
        assert self._capabilities is not None
        with KataTestContainer(
            requirements,
            self._runner,
            self._capabilities,
        ) as vm:
            # bpf-copy attaches TLS uprobes to a library only after the first traced process maps it, so
            # that process may finish its handshake first (the product allows this first-discovery gap).
            # One traced, unchecked exchange attaches them; the checked cells then cover every process.
            # The fixture loads its libssl through LD_LIBRARY_PATH, which exec-time probe planning ignores;
            # drop this exchange once the daemon honours LD_LIBRARY_PATH there.
            tls_case = next((case for case in DATA_CASES if case.needs_tls), None)
            if tls_case is not None:
                self._context.report_progress(
                    f"{_progress_backend(backend.name)}.data.tls",
                    "running the unchecked first TLS discovery exchange",
                )
                try:
                    self._run_data_case(vm, backend.name, tls_case, discovery_only=True)
                except Exception as error:
                    results["tls_discovery"] = TestResult(
                        TestStatus.FAILED,
                        self._failure_diagnostics(vm, error),
                    )
            for case in DATA_CASES:
                self._context.report_progress(
                    f"{_progress_backend(backend.name)}.data."
                    f"{_progress_data_case(case.name)}",
                    "running in reusable data VM",
                )
                try:
                    self._run_data_case(vm, backend.name, case)
                except Exception as error:
                    results[case.name] = TestResult(
                        TestStatus.FAILED,
                        self._failure_diagnostics(vm, error),
                    )
                else:
                    results[case.name] = TestResult(TestStatus.PASSED, "completed")
            # Checked while the VM is alive: the exporter still owns its queue.
            collector = self._collector_evidence(backend.name, started_unix_nano)
            if collector is not None:
                results["collector_egress"] = collector
        return TestResult(
            TestStatus.COMPOSITE,
            "three data cells in one Kata VM",
            results,
        )

    def _collector_evidence(
        self,
        backend: str,
        since_unix_nano: int,
    ) -> TestResult | None:
        traces_file = self._config.collector_traces_file
        if traces_file is None:
            return None
        deployment = self._deployment
        if deployment is None or not deployment.otel_export_enabled:
            return TestResult(
                TestStatus.FAILED,
                "VIRTUAL_CONTAINER_E2E_COLLECTOR_TRACES_FILE is set, but the "
                "Guest artifact was prepared without --otel-endpoint",
            )
        self._context.report_progress(
            f"{_progress_backend(backend)}.data.collector",
            f"waiting for spans in {traces_file}",
        )
        try:
            evidence = wait_for_spans(
                traces_file,
                since_unix_nano,
                self._config.collector_timeout_seconds,
            )
        except RuntimeError as error:
            hint = ""
            if (
                deployment.egress_mode == "network"
                and self._config.network_namespace_helper is None
            ):
                hint = (
                    "; network egress needs a Guest interface, set "
                    "VIRTUAL_CONTAINER_E2E_NETWORK_NAMESPACE_HELPER"
                )
            return TestResult(TestStatus.FAILED, f"{error}{hint}")
        return TestResult(
            TestStatus.PASSED,
            f"{evidence.spans} span(s) in {evidence.traces} trace(s) reached the "
            f"Host Collector over {deployment.egress_mode}",
        )

    @contextmanager
    def _network_namespace(self) -> Iterator[Path | None]:
        """Give one VM a pre-configured network namespace, as a CRI would."""

        helper = self._config.network_namespace_helper
        if helper is None:
            yield None
            return
        name = f"actrail-v2-{secrets.token_hex(6)}"
        created = self._runner.run(
            [str(helper), "add", name],
            timeout=self._config.command_timeout_seconds,
        )
        lines = created.stdout.strip().splitlines()
        if created.returncode != 0 or not lines:
            raise RuntimeError(
                f"network namespace helper failed exit={created.returncode}: "
                + (created.diagnostic or "no namespace path was printed")
            )
        try:
            yield Path(lines[-1])
        finally:
            removed = self._runner.run(
                [str(helper), "del", name],
                timeout=self._config.command_timeout_seconds,
            )
            if removed.returncode != 0:
                self._context.output.line(
                    f"network namespace helper could not remove {name}: "
                    + removed.diagnostic
                )

    def _run_data_case(
        self,
        vm: KataTestContainer,
        backend: str,
        case: DataCase,
        *,
        discovery_only: bool = False,
    ) -> None:
        cell = f"{case.name}-discovery" if discovery_only else case.name
        title = self._trace_title(backend, f"data-{cell}")
        server = None
        if case.needs_tls:
            server = vm.start_exec(
                (
                    "/bin/sh",
                    "-ec",
                    f"(printf 'SERVER_REPLY_9931\\n'; sleep {CASE_CONSTANTS.tls_server_hold_seconds}) | "
                    "OPENSSL_CONF=/opt/actrail-test/openssl.cnf "
                    "LD_LIBRARY_PATH=/opt/actrail-test/lib "
                    "/opt/actrail-test/openssl s_server -accept 4433 -naccept 1 "
                    "-cert /tmp/actrail-tls/cert.pem "
                    "-key /tmp/actrail-tls/key.pem -quiet",
                ),
                uid=self._config.workload_uid,
                gid=self._config.workload_gid,
            )
            time.sleep(CASE_CONSTANTS.server_start_settle_seconds)
            if server.poll() is not None:
                failed = server.wait(timeout=CASE_CONSTANTS.server_exit_timeout_seconds)
                raise RuntimeError(
                    "TLS server exited before client launch: " + failed.diagnostic
                )

        workload = self._data_workload(case)
        launch = vm.exec(
            (
                "/bin/sh",
                "/opt/actrail/bin/actrail-init",
                "--name",
                title,
                "--",
                "/bin/sh",
                "-c",
                workload,
            ),
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            timeout=self._config.runtime_timeout_seconds,
        )
        if launch.returncode != 0:
            raise RuntimeError(
                f"guest-root workload launch failed exit={launch.returncode}: "
                + launch.diagnostic
            )
        (self._tmp_dir / f"{backend}-{cell}-launch.txt").write_text(launch.stdout + launch.stderr, encoding="utf-8")
        if server is not None:
            server_result = server.wait(timeout=CASE_CONSTANTS.server_completion_timeout_seconds)
            if server_result.returncode != 0:
                raise RuntimeError(
                    f"TLS server failed exit={server_result.returncode}: "
                    + server_result.diagnostic
                )
        if self._config.settle_seconds:
            time.sleep(self._config.settle_seconds)
        trace_id = self._wait_for_clean_trace(vm, title)
        if discovery_only:
            # Kept as evidence only: a missing first exchange here is the allowed discovery gap.
            payloads = self._guest_command(
                vm,
                "/usr/local/bin/actrailviewer --config /etc/actrail/operator.conf "
                f"payloads --trace-id {trace_id}",
                check=False,
            )
            (self._tmp_dir / f"{backend}-{cell}-payloads.txt").write_text(payloads.stdout, encoding="utf-8")
            return
        summary = self._guest_command(
            vm,
            "/usr/local/bin/actrailviewer --config /etc/actrail/operator.conf "
            f"summary --trace-id {trace_id}",
        )
        counts = parse_summary_counts(summary.stdout)
        (self._tmp_dir / f"{backend}-{case.name}-summary.txt").write_text(summary.stdout, encoding="utf-8")
        require_markers(
            launch.stdout + launch.stderr,
            ("deployment_permissions_degraded=false",),
            context=f"{case.name} launch",
        )
        if case.needs_ebpf:
            require_markers(
                launch.stdout + launch.stderr,
                (
                    "deployment_permissions_selected="
                    "host_ebpf:enabled,seccomp_notify:disabled",
                ),
                context=f"{case.name} launch",
            )
            if counts.events <= 0:
                raise RuntimeError(f"{case.name} retained zero eBPF events")
        if case.needs_tls:
            payloads = self._guest_command(
                vm,
                "/usr/local/bin/actrailviewer --config "
                "/etc/actrail/operator.conf "
                f"payloads --trace-id {trace_id}",
            )
            require_markers(
                payloads.stdout,
                ("TlsUserSpace", "SSL_write", "SSL_read", "26/26", "18/18"),
                context=f"{case.name} payloads",
            )
            (self._tmp_dir / f"{backend}-{case.name}-payloads.txt").write_text(payloads.stdout, encoding="utf-8")
            for function, expected_hex in (("SSL_write", "434c49454e545f5345435245545f4d41524b45525f373738380a"),
                                           ("SSL_read", "5345525645525f5245504c595f393933310a")):
                segment = next(line.split()[0] for line in payloads.stdout.splitlines() if "TlsUserSpace" in line and function in line)
                content = self._guest_command(vm, "/usr/local/bin/actrailviewer --config /etc/actrail/operator.conf "
                    f"payload --trace-id {shlex.quote(trace_id)} --segment-id {shlex.quote(segment)} --format hex")
                (self._tmp_dir / f"{backend}-{case.name}-{function}.hex").write_text(content.stdout, encoding="utf-8")
                if content.stdout.strip() != expected_hex:
                    raise RuntimeError(f"{case.name} {function} plaintext content differs from the fixed fixture")

    def _requirements(
        self,
        backend: str,
        runtime_config: Path,
        *,
        profile_name: str,
        data: bool,
        network_namespace: Path | None = None,
    ) -> KataContainerRequirements:
        mounts = [
            KataMount(self._workload_bundle, "/opt/actrail", read_only=True),
            KataMount(Path("/dev/actrail"), "/run/actrail", read_only=True),
        ]
        artifact_directories = [self._workload_bundle]
        main_command: tuple[str, ...] = ("/bin/sh", "-c", f"sleep {CASE_CONSTANTS.vm_hold_seconds}")
        if data:
            mounts.append(
                KataMount(self._guest_bundle, "/opt/actrail-test", read_only=True)
            )
            artifact_directories.append(self._guest_bundle)
            main_command = (
                "/bin/sh",
                "-ec",
                "mkdir -p /tmp/actrail-tls; "
                "OPENSSL_CONF=/opt/actrail-test/openssl.cnf "
                "LD_LIBRARY_PATH=/opt/actrail-test/lib "
                "/opt/actrail-test/openssl req -x509 -newkey rsa:2048 "
                "-keyout /tmp/actrail-tls/key.pem "
                "-out /tmp/actrail-tls/cert.pem -days 1 -nodes "
                "-subj /CN=localhost >/tmp/actrail-tls/cert.log 2>&1; "
                f"sleep {CASE_CONSTANTS.vm_hold_seconds}",
            )
        else:
            mounts.append(
                KataMount(self._case_dir, "/opt/actrail-test", read_only=True)
            )
        builder = KataRequirementsBuilder(
            backend=backend,
            runtime=self._config.ctr_runtime,
            runtime_config=runtime_config,
            image=self._config.image,
            runner=self._runner,
            pull_policy=self._config.image_pull_policy,
            image_archive=self._config.image_archive,
            runtime_timeout_seconds=self._config.runtime_timeout_seconds,
            uid=self._config.workload_uid,
            gid=self._config.workload_gid,
            ready_timeout_seconds=self._config.ready_timeout_seconds,
        )
        return builder.build(
            name_prefix=f"virtual-{backend}-{profile_name}",
            command=main_command,
            mounts=tuple(mounts),
            artifact_directories=tuple(artifact_directories),
            labels=(("io.actrail.test.profile", profile_name),),
            running_validator=lambda vm: self._validate_workload_ready(vm, data=data),
            network_namespace=network_namespace,
        )

    def _validate_workload_ready(
        self,
        vm: KataTestContainer,
        *,
        data: bool,
    ) -> RequirementCheck:
        if not vm.is_running():
            return RequirementCheck.not_ready(
                "workload task exited before readiness",
                refreshable=True,
            )
        required_asset = (
            "test -x /opt/actrail-test/openssl; command -v base64 >/dev/null; "
            if data
            else ""
        )
        result = vm.exec(
            (
                "/bin/sh",
                "-ec",
                ". /etc/os-release; [ \"${ID:-}\" = openEuler ]; "
                "command -v /usr/bin/setpriv >/dev/null; "
                f"{required_asset}"
                f"remaining={CASE_CONSTANTS.guest_ready_attempts}; while [ $remaining -gt 0 ]; do "
                "[ -S /run/actrail/control.sock ] && exit 0; "
                f"remaining=$((remaining - 1)); sleep {CASE_CONSTANTS.guest_ready_poll_seconds}; done; exit 72",
            ),
            timeout=self._config.ready_timeout_seconds,
        )
        if result.returncode == 0:
            return RequirementCheck.ready_check()
        diagnostic = result.diagnostic or "workload readiness command failed"
        non_refreshable = any(
            marker in diagnostic.lower()
            for marker in ("permission denied", "operation not permitted", "kvm")
        )
        return RequirementCheck.not_ready(
            diagnostic,
            refreshable=not non_refreshable,
        )

    def _wait_for_clean_trace(self, vm: KataTestContainer, title: str) -> str:
        deadline = time.monotonic() + self._config.ready_timeout_seconds
        last_error = "trace was not listed"
        while time.monotonic() < deadline:
            traces = self._guest_command(
                vm,
                "/usr/local/bin/actrailviewer --config "
                "/etc/actrail/operator.conf --output-format json traces",
                check=False,
            )
            if traces.returncode == 0:
                try:
                    trace_id = find_clean_trace(traces.stdout, title).trace_id
                    (self._tmp_dir / f"{title}-traces.json").write_text(traces.stdout, encoding="utf-8")
                    return trace_id
                except RuntimeError as error:
                    last_error = str(error)
            else:
                last_error = traces.diagnostic
            time.sleep(CASE_CONSTANTS.wait_for_clean_trace_poll_seconds)
        raise RuntimeError(f"timed out waiting for clean trace {title}: {last_error}")

    def _guest_command(
        self,
        vm: KataTestContainer,
        command: str,
        *,
        check: bool = True,
    ) -> CommandResult:
        result = self._guest.capture(
            vm.container_id,
            command,
            timeout=self._config.runtime_timeout_seconds,
        )
        if check and result.returncode != 0:
            raise RuntimeError(
                f"guest command failed exit={result.returncode}: "
                + result.diagnostic
            )
        return result

    def _data_workload(self, case: DataCase) -> str:
        if case.name == "tls-only":
            return (
                "printf 'CLIENT_SECRET_MARKER_7788\\n' | "
                "OPENSSL_CONF=/opt/actrail-test/openssl.cnf "
                "LD_LIBRARY_PATH=/opt/actrail-test/lib "
                "/opt/actrail-test/openssl s_client "
                "-connect 127.0.0.1:4433 -quiet"
            )
        if case.name == "ebpf-only":
            return (
                "mkdir -p /tmp/actrail-ebpf; "
                "printf 'GUEST_ROOT_EBPF_MARKER\\n' >/tmp/actrail-ebpf/source; "
                "cp /tmp/actrail-ebpf/source /tmp/actrail-ebpf/copy; "
                "rm -f /tmp/actrail-ebpf/source /tmp/actrail-ebpf/copy"
            )
        return (
            "mkdir -p /tmp/actrail-combo; "
            "printf 'GUEST_ROOT_COMBO_MARKER\\n' >/tmp/actrail-combo/source; "
            "cp /tmp/actrail-combo/source /tmp/actrail-combo/copy; "
            "printf 'CLIENT_SECRET_MARKER_7788\\n' | "
            "OPENSSL_CONF=/opt/actrail-test/openssl.cnf "
            "LD_LIBRARY_PATH=/opt/actrail-test/lib "
            "/opt/actrail-test/openssl s_client "
            "-connect 127.0.0.1:4433 -quiet; "
            "rm -f /tmp/actrail-combo/source /tmp/actrail-combo/copy"
        )

    def _base_environment(self) -> dict[str, str]:
        environment = os.environ.copy()
        environment.pop("RUNTIME_CONFIG_PATH", None)
        environment.update(
            {
                "ACTRAIL_REPO_ROOT": str(self._config.repo),
                "ACTRAIL_BIN_DIR": str(self._config.bin_dir),
                "ACTRAIL_TEST_WORK_DIR": str(self._config.work_dir),
                "TMPDIR": str(self._tmp_dir),
                "KATA_CONFIG_DIRS": os.pathsep.join(
                    str(path) for path in self._config.kata_config_dirs
                ),
            }
        )
        return environment

    def _failure_diagnostics(
        self,
        vm: KataTestContainer,
        error: Exception,
    ) -> str:
        details = [str(error), vm.diagnostics()]
        try:
            guest = self._guest.capture(
                vm.container_id,
                "systemctl --no-pager --full status actraild.service || true; "
                "journalctl --no-pager -u actraild.service -n 100 || true; "
                "ls -ld /sys/kernel/btf/vmlinux /sys/kernel/tracing "
                "/sys/kernel/debug/tracing /sys/fs/bpf /dev/actrail "
                "/run/actrail 2>&1 || true",
                timeout=self._config.ready_timeout_seconds,
            )
        except Exception as diagnostic_error:
            details.append(f"guest diagnostics failed: {diagnostic_error}")
        else:
            details.append("guest diagnostics:\n" + guest.stdout.strip())
        diagnostic = "\n".join(detail for detail in details if detail)
        self._context.output.line(diagnostic)
        return diagnostic

    def _run_command(
        self,
        command: list[str],
        *,
        environment: dict[str, str],
        timeout: float,
    ) -> TestResult:
        self._context.output.line("+ " + " ".join(command))
        try:
            result = self._runner.run(
                command,
                cwd=self._config.repo,
                environment=environment,
                timeout=timeout,
            )
        except CommandTimeoutError as error:
            self._context.output.command_output(
                error.result.stdout,
                error.result.stderr,
            )
            return TestResult(TestStatus.FAILED, f"timed out after {timeout}s")
        except OSError as error:
            return TestResult(TestStatus.FAILED, str(error))
        self._context.output.command_output(result.stdout, result.stderr)
        if result.returncode != 0:
            status = f"exit status {result.returncode}"
            diagnostic = result.diagnostic
            return TestResult(
                TestStatus.FAILED,
                f"{status} — {diagnostic}" if diagnostic else status,
            )
        return TestResult(TestStatus.PASSED, "completed")

    @staticmethod
    def _trace_title(backend: str, cell: str) -> str:
        return f"kata-{backend}-{cell}-{secrets.token_hex(4)}"


def _progress_backend(backend: str) -> str:
    return {
        "cloud-hypervisor": "cloud",
        "stratovirt": "strato",
    }[backend]


def _progress_data_case(case: str) -> str:
    return {
        "tls-only": "tls",
        "ebpf-only": "ebpf",
    }.get(case, case)
