from __future__ import annotations

import errno
import json
import os
import re
import shlex
import shutil
import time
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.core import CommonTestConfig, TestCase, TestCaseInputs, TestResult, TestStatus, has_failure
from tests.v2.common.runner import TestingContextSingleton

from .dynamic import DynamicPolicies
from .routing import BackendSelection
from .services import TcpWitness, WorkloadProvider


class LsmAccessControlXiaooCase(TestCase):
    def __init__(self, inputs: TestCaseInputs):
        self._config = CommonTestConfig.from_environment(inputs, "LSM_ACCESS_CONTROL_XIAOO")
        executable = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_BINARY") or shutil.which("xiaoo")
        if executable is None:
            raise RuntimeError("real Xiaoo executable is not available")
        self._xiaoo = Path(executable).resolve()
        self._timeout = float(os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_READY_TIMEOUT_SECONDS", "15"))
        self._poll = float(os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_PEER_POLL_SECONDS", "0.1"))
        if self._timeout <= 0 or self._poll <= 0:
            raise ValueError("LSM readiness and peer poll timeouts must be positive")
        self._selection = BackendSelection("lsm")
        self._ipv6 = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_IPV6") == "1"
        self._alerts = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_ALERTS") == "1"
        self._paths = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_PATHS") == "1"
        if sum((self._ipv6, self._alerts, self._paths)) > 1:
            raise ValueError("IPv6, boundary alert and path scenarios are mutually exclusive")
        self._audit_enabled = True
        self._alert_window_ms = 60_000
        self._alert_max_groups = 256
        self._deny_on_error = True
        self._capacity = int(os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_POLICY_MAX_ENTRIES", "64"))
        if not 32 <= self._capacity <= 128:
            raise ValueError("LSM test policy capacity must be between 32 and 128")
        self._work = self._config.work_dir
        if any(character.isspace() for character in str(self._work)):
            raise ValueError("static policy grammar requires a work directory without whitespace")
        self._marker = self._work / "protected/file.marker"
        self._mkdir = self._work / "create-target"
        self._rmdir = self._work / "remove-target"
        self._executable = self._work / "controlled-true"
        self._results = self._work / "workload.json"
        self._patch = self._work / "actraild.patch.toml"
        self._xiaoo_config = self._work / "xiaoo.toml"
        self._runtime: ActrailRuntime | None = None
        self._provider: WorkloadProvider | None = None
        self._dynamic: DynamicPolicies | None = None
        self._peer = TcpWitness(self._timeout, self._poll, ipv6=self._ipv6)

    def run(self, context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        try:
            status = Path("/proc/self/status").read_text(encoding="utf-8")
            if not re.search(r"^Seccomp:\s+0$", status, re.MULTILINE):
                raise RuntimeError("this scenario requires an environment with inherited Seccomp=0")
            self._marker.parent.mkdir()
            self._marker.write_text("", encoding="utf-8")
            true_binary = shutil.which("true")
            if true_binary is None:
                raise RuntimeError("real true ELF executable is not available")
            shutil.copy2(Path(true_binary).resolve(), self._executable)
            port = self._peer.start()
            self._provider = WorkloadProvider(
                self._config.repo, self._work, self._workload_command(port), self._timeout
            )
            provider_url = self._provider.start()
            self._xiaoo_config.write_text(
                '[llm]\nprovider = "deepseek"\nmodel = "deepseek-chat"\n'
                'api_key_env = "ACTRAIL_XIAOO_LSM_KEY"\n'
                f"api_base = {json.dumps(provider_url)}\n"
                'max_tokens = 128\ncontext_window = 32768\nreasoning_effort = "off"\n',
                encoding="utf-8",
            )
            selections = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS", ",".join(BackendSelection.MATRIX)).split(",")
            for selection in selections:
                self._selection = BackendSelection(selection.strip())
                if self._paths:
                    if self._selection.name not in ("lsm", "fanotify_seccomp"):
                        raise ValueError("path coverage requires lsm or fanotify_seccomp")
                    from .paths import PathBindingScenario
                    PathBindingScenario(self).run(context, results, port)
                    continue
                if self._alerts:
                    if self._selection.name != "lsm" or self._ipv6:
                        raise ValueError("boundary alert coverage requires LSM without IPv6 mode")
                    from .alerts import BoundaryAlertsScenario
                    BoundaryAlertsScenario(self).run(context, results, port)
                    continue
                if self._ipv6:
                    if self._selection.name != "lsm":
                        raise ValueError("IPv6 short sockaddr coverage requires the LSM backend")
                    self._run_ipv6(context, results, port)
                    continue
                scenario = {}
                results[selection] = TestResult(TestStatus.COMPOSITE, selection, scenario)
                self._write_patch()
                self._runtime = ActrailRuntime(
                    self._config.repo, self._config.bin_dir, self._config.command_timeout_seconds,
                    context.output, self._work / "actraild.conf", self._patch,
                )
                for decision in ("deny", "allow"):
                    context.report_progress(
                        f"xiaoo_{decision}", f"{selection}: refreshing defaults and checking file/exec/connect {decision}"
                    )
                    self._write_rules(decision, port)
                    self._results.unlink(missing_ok=True)
                    self._runtime.prepare()
                    trace_id = self._run_xiaoo(decision)
                    self._require_workload(decision)
                    self._require_evidence(trace_id, decision)
                    stopped = self._runtime.stop()
                    if stopped is not None and stopped.returncode != 0:
                        raise RuntimeError(f"daemon stop failed: {stopped.output}")
                    scenario[decision] = TestResult(
                        TestStatus.PASSED,
                        f"trace-{trace_id}: real file/exec/connect {decision}; selected backend audits, resources and bpf-copy verified",
                    )
                self._run_dynamic(context, scenario, port)
                assert self._dynamic is not None
                cleaned = self._dynamic.cleanup()
                if has_failure(cleaned):
                    raise RuntimeError(f"{selection} cleanup failed: {cleaned}")
                self._dynamic = None
                self._runtime = None
            return TestResult(TestStatus.COMPOSITE, "real Xiaoo per-operation access control", results)
        except Exception as error:
            self._print_daemon_diagnostics()
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(TestStatus.COMPOSITE, "real Xiaoo per-operation access control", results)

    def _print_daemon_diagnostics(self) -> None:
        log = self._work / "log/actraild.log"
        if log.is_file():
            limit = int(os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_DIAGNOSTIC_BYTES", "65536"))
            with log.open("rb") as stream:
                stream.seek(0, os.SEEK_END)
                stream.seek(max(0, stream.tell() - max(1, limit)))
                print("LSM daemon diagnostics before cleanup:\n" + stream.read().decode(errors="replace"))

    def _workload_command(self, port: int) -> str:
        python = shutil.which("python3")
        if python is None:
            raise RuntimeError("Python 3 workload interpreter is unavailable")
        return shlex.join([
            str(Path(python).resolve()), str(Path(__file__).with_name("workload.py")),
            "--marker", str(self._marker), "--executable", str(self._executable),
            "--results", str(self._results), "--xiaoo", str(self._xiaoo),
            "--daemon-pid", str(self._work / "run/actraild.pid"),
            "--mkdir", str(self._mkdir), "--rmdir", str(self._rmdir),
            "--port", str(port), "--timeout", str(self._timeout),
            *(["--ipv6"] if self._ipv6 else []),
            *(["--denial-repetitions", "64", "--denial-pause", "0.01"] if self._alerts else []),
            *(["--path-probes", str(self._work / "path-probes.json")] if self._paths else []),
        ])

    def _write_rules(self, decision: str, port: int) -> None:
        remote = f"[::1]:{port}" if self._ipv6 else "127.0.0.2:*"
        for domain, content in {
            "file": f"lsm-file-{decision} {decision} open {self._marker}\n",
            "command": (
                f"lsm-command-{decision} {decision} exec {self._executable} priority 0\n"
                f"lsm-root allow exec {self._xiaoo} priority 0\n"
                f"lsm-future deny exec {self._work / 'future-true'} priority 0\n"
            ),
            "network": f"lsm-network-{decision} {decision} connect {remote}\n",
        }.items():
            (self._work / f"{domain}.rules").write_text(content, encoding="utf-8")

    def _run_xiaoo(self, decision: str) -> int:
        assert self._runtime is not None
        self._marker.write_text("", encoding="utf-8")
        if self._mkdir.exists():
            self._mkdir.rmdir()
        self._rmdir.mkdir(exist_ok=True)
        self._results.unlink(missing_ok=True)
        self._peer.requests.clear()
        environment = os.environ.copy()
        for name in ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"):
            environment.pop(name, None)
        environment.update(
            NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost",
            ACTRAIL_XIAOO_LSM_KEY="local-test-key", PATH="/usr/bin",
        )
        result = self._runtime.run(
            self._runtime.control_command(
                "launch", "--seccomp-notify", "required" if self._selection.seccomp else "disabled", "--name", f"v2-lsm-xiaoo-{decision}",
                "--", self._xiaoo, "--cli", "run", "--config", self._xiaoo_config,
                "--tools", "bash", "--max-turns", "3", "--debug",
                "--prompt", "Use the Bash tool exactly once, then report its operating-system results.",
            ),
            timeout_seconds=self._config.launch_timeout_seconds, environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(f"Xiaoo failed: {result.output[-4000:]}")
        match = re.search(r"trace trace-(\d+) entered Active", result.output)
        if match is None:
            raise AssertionError("Xiaoo launch omitted trace ID")
        for capability in [*self._selection.capabilities, "socket-plaintext-payload"]:
            if capability not in result.output:
                raise AssertionError(f"launch omitted capability {capability}")
        return int(match.group(1))

    def _require_workload(
        self, decision: str, *, directories: bool = False, network_decision: str | None = None,
    ) -> None:
        result = json.loads(self._results.read_text(encoding="utf-8"))
        for domain in ("file", "command", "network", "mkdir", "rmdir"):
            expected = decision if directories or domain not in ("mkdir", "rmdir") else "allow"
            if domain == "network" and network_decision is not None:
                expected = network_decision
            observed = result[domain]
            if observed["allowed"] != (expected == "allow"):
                raise AssertionError(f"{domain} {expected} failed: {observed}")
            if expected == "deny" and observed.get("errno") not in (errno.EACCES, errno.EPERM):
                raise AssertionError(f"{domain} failure was not a policy rejection: {observed}")
        directories_allowed = not directories or decision == "allow"
        if self._mkdir.exists() != directories_allowed or self._rmdir.exists() == directories_allowed:
            raise AssertionError("mkdir/rmdir filesystem effects differ from their decisions")
        expected_marker = "ACTRAIL_LSM_FILE" if decision == "allow" else ""
        if self._marker.read_text(encoding="utf-8") != expected_marker:
            raise AssertionError(f"file {decision} had an unexpected filesystem effect")
        expected_requests = [b"ACTRAIL_LSM_NETWORK"] if (network_decision or decision) == "allow" else []
        if self._peer.requests != expected_requests or self._peer.failure:
            raise AssertionError(f"network {decision} reached an unexpected peer state: {self._peer.requests}")
        self._selection.require_resources(result)
        if str(self._xiaoo) not in {process["executable"] for process in result["processes"]}:
            raise AssertionError("mechanism verification did not include the actual Xiaoo process")

    def _require_evidence(
        self, trace_id: int, decision: str, rules: dict[str, str] | None = None,
    ) -> None:
        assert self._runtime is not None
        required = dict(rules) if rules is not None else {
            f"lsm-{domain}-{decision}": decision for domain in ("file", "command", "network")
        }
        required["lsm-root"] = "allow"
        deadline = time.monotonic() + self._timeout
        missing = set(required)
        has_payload = False
        while time.monotonic() < deadline:
            self._runtime.run_checked(self._runtime.control_command("list-traces"), echo=False)
            events = self._runtime.run_checked(self._runtime.viewer_command(
                "events", "--trace-id", str(trace_id), "--output-format", "json"
            ), echo=False)
            for event in json.loads(events.stdout)["events"]:
                for rule_id in tuple(missing):
                    if self._selection.audit_matches(event, rule_id, required[rule_id]):
                        missing.discard(rule_id)
            payloads = self._runtime.run_checked(self._runtime.viewer_command(
                "payloads", "--trace-id", str(trace_id), "--output-format", "json"
            ), echo=False)
            has_payload = any(
                payload.get("captured_size", 0) > 0
                for payload in json.loads(payloads.stdout)["payloads"]
            )
            if not missing and has_payload:
                return
            time.sleep(self._config.drain_interval_seconds)
        print(events.stdout)
        print(payloads.stdout)
        raise AssertionError(f"LSM audit missing={sorted(missing)}, captured bpf-copy payload={has_payload}")

    def _run_dynamic(self, context: TestingContextSingleton, results: dict, port: int) -> None:
        context.report_progress("dynamic_prepare", "loading the real managed file/command/network policy plugins")
        for domain in ("file", "network"):
            (self._work / f"{domain}.rules").write_text("", encoding="utf-8")
        (self._work / "command.rules").write_text(
            f"lsm-root allow exec {self._xiaoo} priority 0\n"
            f"lsm-retained deny exec {self._work / 'retained-true'} priority 0\n", encoding="utf-8",
        )
        retained = self._work / "retained-true"
        shutil.copy2(self._executable, retained)
        self._dynamic = DynamicPolicies(self._config, context.output, port)
        self._runtime = self._dynamic.runtime
        self._dynamic.prepare()
        retained.unlink()
        if self._mkdir.exists():
            self._mkdir.rmdir()
        # Publish new executable rules before the target exists, with an old
        # static executable also deleted. All later updates and owner revocations
        # must remain available while the retained target stays absent.
        replacement = self._executable.with_suffix(".saved")
        self._executable.rename(replacement)
        try:
            required = self._dynamic.publish("deny")
        finally:
            replacement.rename(self._executable)
        self._dynamic_workload("dynamic_deny", "deny", required, context, results)
        rejections = (
            ("argv_rejection", self._dynamic.require_argv_rejected),
            ("capacity_rejection", lambda: self._dynamic.require_capacity_rejected(self._capacity)),
        ) if self._selection.name == "lsm" else ()
        for name, reject in rejections:
            context.report_progress(name, "rejecting invalid publication and checking the current policy")
            reject()
            if name == "argv_rejection":
                self._dynamic.require_identifier_rejected()
            self._dynamic_workload(name, "deny", required, context, results)
        if self._selection.name == "fanotify_seccomp":
            required = self._dynamic.publish("gray")
            self._dynamic_workload("dynamic_gray_unavailable", "deny", required, context, results)
        if self._selection.name in ("lsm", "fanotify_seccomp"):
            for decision in ("allow", "deny"):
                required = self._dynamic.publish_network_exception(decision)
                self._dynamic_workload(f"network_exact_{decision}", decision, required, context, results)
            self._dynamic.require_network_owner_conflicts()
            self._dynamic_workload("network_owner_conflicts", "deny", required, context, results)
        required = self._dynamic.publish("allow")
        self._dynamic_workload("dynamic_allow", "allow", required, context, results)
        self._dynamic.publish("deny")
        self._dynamic.unload_all()
        self._dynamic_workload("owner_unload", "allow", {}, context, results)

    def _dynamic_workload(
        self, name: str, decision: str, rules: dict[str, str],
        context: TestingContextSingleton, results: dict,
    ) -> None:
        context.report_progress(name, f"real Xiaoo verifies file/exec/connect/mkdir/rmdir {decision}")
        trace_id = self._run_xiaoo(name)
        self._require_workload(decision, directories=True)
        self._require_evidence(trace_id, decision, rules)
        results[name] = TestResult(
            TestStatus.PASSED,
            f"trace-{trace_id}: actual file/exec/connect/mkdir/rmdir {decision}, selected backend audit and bpf-copy verified",
        )

    def _run_ipv6(self, context: TestingContextSingleton, results: dict, port: int) -> None:
        for deny_on_error in (True, False):
            self._deny_on_error = deny_on_error
            self._write_patch()
            self._runtime = ActrailRuntime(
                self._config.repo, self._config.bin_dir, self._config.command_timeout_seconds,
                context.output, self._work / "actraild.conf", self._patch,
            )
            for decision in ("allow", "deny"):
                name = f"ipv6_24_{decision}_deny_on_error_{deny_on_error}"
                context.report_progress(name, "fresh defaults; real Xiaoo connects with a 24-byte IPv6 sockaddr")
                self._write_rules(decision, port)
                self._runtime.prepare()
                trace_id = self._run_xiaoo(name)
                self._require_workload(decision)
                self._require_evidence(trace_id, decision)
                stopped = self._runtime.stop()
                if stopped is not None and stopped.returncode != 0:
                    raise RuntimeError(f"daemon stop failed: {stopped.output}")
                results[name] = TestResult(
                    TestStatus.PASSED, f"trace-{trace_id}: IPv6 {decision}, peer effects and rule audit verified",
                )

    def _write_patch(self) -> None:
        lsm_enabled = "lsm" in self._selection.backends.values()
        ActrailRuntime.write_isolated_operator_config_patch(
            self._patch, self._work, payload_tls_enabled=False, payload_stdio_enabled=False,
        )
        document = self._patch.read_text(encoding="utf-8")
        document = document.replace("[payload.tls]\n", '[payload.tls]\ncapture_backend = "bpf-copy"\n')
        document += (
            '\n[payload.socket]\nenabled = true\ncapture_backend = "bpf-copy"\n'
            '\n[semantic_retention.l4_payload]\nenabled = true\nbody_content = "retained"\n'
            '\n[ebpf]\nenabled = true\n'
            '\n[process_seccomp]\nenabled = false\n'
            '\n[access_control.backends.lsm]\ndeny_signal = "none"\n'
            f"deny_on_error = {str(self._deny_on_error).lower()}\n"
            f"alert_window_ms = {self._alert_window_ms if lsm_enabled else 0}\n"
            f"alert_max_groups = {self._alert_max_groups if lsm_enabled else 0}\n"
            f"policy_max_entries = {self._capacity if lsm_enabled else 0}\n"
            + ("" if lsm_enabled else "path_max_bytes = 0\naudit_buffer_bytes = 0\naudit_batch_max = 0\n") +
            '\n[plugins.startup]\nenabled = false\n'
        )
        document += self._selection.config()
        for section, domain in (("enforcement", "file"), ("command_control", "command"), ("network_control", "network")):
            document += (
                f"\n[{section}]\nenabled = true\n"
                f"rules_path = {json.dumps(str(self._work / f'{domain}.rules'))}\n"
                'default_decision = "allow"\n'
                f"audit_enabled = {str(self._audit_enabled).lower()}\n"
            )
        self._patch.write_text(document, encoding="utf-8")

    def cleanup(self, context: TestingContextSingleton) -> TestResult:
        del context
        failures = []
        try:
            if self._dynamic is not None:
                result = self._dynamic.cleanup()
                if has_failure(result):
                    failures.append(str(result))
            elif self._runtime is not None:
                result = self._runtime.stop()
                if result is not None and result.returncode != 0:
                    failures.append(result.output)
        except Exception as error:
            failures.append(str(error))
        for service in (self._provider, self._peer):
            if service is not None:
                try:
                    service.stop()
                except Exception as error:
                    failures.append(str(error))
        return TestResult(
            TestStatus.FAILED if failures else TestStatus.PASSED,
            "; ".join(failures) if failures else "LSM daemon, provider, and TCP witness stopped",
        )
