from __future__ import annotations

import json
import os
import re
import shutil
import sqlite3
import time
import tomllib
from contextlib import closing
from pathlib import Path

from tests.v2.common.actrail_runtime import ActrailRuntime
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.core import (
    CommonTestConfig,
    TestCase,
    TestCaseInputs,
    TestResult,
    TestStatus,
)
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.common.resource_storage_assertion import assert_host_columns, storage_counts
from tests.v2.regression.command_policy_xiaoo.provider import LocalXiaooProvider

from .constants import CASE_CONSTANTS


class FilePolicyXiaooCase(TestCase):
    """Exercise default complete LSM control and explicit fanotify file control."""

    def __init__(self, inputs: TestCaseInputs):
        self._config = CommonTestConfig.from_environment(inputs, "FILE_POLICY_XIAOO")
        executable = os.environ.get("FILE_POLICY_XIAOO_E2E_BINARY") or shutil.which("xiaoo")
        if executable is None:
            raise RuntimeError("real Xiaoo executable is not available")
        self._xiaoo = Path(executable).resolve()
        self._timeout = float(os.environ.get("FILE_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS", str(CASE_CONSTANTS.ready_timeout_seconds)))
        if self._timeout <= 0:
            raise ValueError("file-policy readiness timeout must be positive")
        self._variant = "default_complete"
        self._marker = self._config.work_dir / "xiaoo-file.marker"
        self._rules = self._config.work_dir / "file-control.rules"
        self._patch = self._config.work_dir / "actraild.patch.toml"
        self._xiaoo_config = self._config.work_dir / "xiaoo.toml"
        self._runtime: ActrailRuntime | None = None
        self._provider = LocalXiaooProvider(
            self._config.repo, self._config.work_dir, self._marker, self._timeout
        )

    def run(self, context: TestingContextSingleton) -> TestResult:
        results: dict[str, TestResult] = {}
        current = "setup"
        try:
            self._marker.write_text("", encoding="utf-8")
            provider_url = self._provider.start()
            self._xiaoo_config.write_text(
                f'[llm]\nprovider = "{CASE_CONSTANTS.llm_provider}"\nmodel = "{CASE_CONSTANTS.llm_model}"\n'
                'api_key_env = "ACTRAIL_XIAOO_COMMAND_POLICY_KEY"\n'
                f"api_base = {json.dumps(provider_url)}\n"
                f'max_tokens = {CASE_CONSTANTS.llm_max_tokens}\ncontext_window = {CASE_CONSTANTS.llm_context_window}\nreasoning_effort = "{CASE_CONSTANTS.llm_reasoning_effort}"\n',
                encoding="utf-8",
            )
            for variant in ("default_complete", "fanotify", "fanotify_shared_storage"):
                self._variant = variant
                self._write_patch()
                self._runtime = ActrailRuntime(
                    self._config.repo,
                    self._config.bin_dir,
                    self._config.command_timeout_seconds,
                    context.output,
                    self._config.work_dir / "actraild.conf",
                    operator_config_patches=self._patches(),
                )
                for decision in ("deny", "allow"):
                    current = f"{variant}_{decision}"
                    self._marker.write_text("", encoding="utf-8")
                    context.report_progress(
                        f"xiaoo_{decision}",
                        f"refreshing default configuration and exercising file {decision}",
                    )
                    self._rules.write_text(
                        f"file-{decision} {decision} open {self._marker}\n",
                        encoding="utf-8",
                    )
                    self._runtime.prepare()
                    self._require_configuration()
                    trace_id = self._run_xiaoo(decision)
                    self._require_evidence(trace_id, decision)
                    stopped = self._runtime.stop()
                    if stopped is not None and stopped.returncode != 0:
                        raise RuntimeError(f"daemon stop failed: {stopped.output}")
                    results[current] = TestResult(
                        TestStatus.PASSED,
                        f"{variant}: real Xiaoo trace-{trace_id} file open {decision} and selected backend audit verified",
                    )
                    current = f"{variant}_{decision}_resource_storage"
                    context.report_progress(current, "checking column storage, host readback and shared trace events")
                    storage = self._require_resource_storage()
                    results[current] = TestResult(
                        TestStatus.PASSED, storage,
                    )
            return TestResult(TestStatus.COMPOSITE, "real Xiaoo file policy", results)
        except Exception as error:
            results[current] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(TestStatus.COMPOSITE, "real Xiaoo file policy", results)

    def _run_xiaoo(self, decision: str) -> int:
        assert self._runtime is not None
        environment = os.environ.copy()
        for name in ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"):
            environment.pop(name, None)
        environment.update(
            NO_PROXY=CASE_CONSTANTS.no_proxy_hosts,
            no_proxy=CASE_CONSTANTS.no_proxy_hosts,
            ACTRAIL_XIAOO_COMMAND_POLICY_KEY="local-test-key",
            PATH="/usr/bin",
        )
        result = self._runtime.run(
            self._runtime.control_command(
                "launch", "--seccomp-notify", "auto" if self._variant == "default_complete" else "disabled",
                "--name", f"v2-file-policy-xiaoo-{decision}", "--",
                self._xiaoo, "--cli", "run", "--config", self._xiaoo_config,
                "--tools", "bash", "--max-turns", str(CASE_CONSTANTS.agent_max_turns), "--debug",
                "--prompt", "Use the Bash tool exactly once, then report its operating-system result.",
            ),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(f"Xiaoo failed: {result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}")
        content = self._marker.read_text(encoding="utf-8")
        permission_error = any(
            message in result.output.lower()
            for message in ("permission denied", "operation not permitted")
        )
        if decision == "deny" and (content or not permission_error):
            raise AssertionError(f"denied open was not enforced: {result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}")
        if decision == "allow" and (content != "ACTRAIL_XIAOO_COMMAND_OK" or permission_error):
            raise AssertionError(f"allowed open did not write its marker: {result.output[-CASE_CONSTANTS.launch_diagnostic_tail_characters:]}")
        match = re.search(r"trace trace-(\d+) entered Active", result.output)
        if match is None:
            raise AssertionError("Xiaoo launch omitted trace ID")
        capability = "enforcement-file-permission-lsm" if self._variant == "default_complete" else "enforcement-file-permission-fanotify"
        if capability not in result.output:
            raise AssertionError("launch omitted file enforcement capability")
        return int(match.group(1))

    def _require_evidence(self, trace_id: int, decision: str) -> None:
        assert self._runtime is not None
        expected = {
            "backend": "bpf-lsm" if self._variant == "default_complete" else "fanotify",
            "operation": "open",
            "rule_id": f"file-{decision}",
            "decision": decision,
            "result": "denied" if decision == "deny" else "allowed",
            "path": str(self._marker),
        }
        deadline = time.monotonic() + self._timeout
        while time.monotonic() < deadline:
            self._runtime.run_checked(self._runtime.control_command("list-traces"), echo=False)
            result = self._runtime.run_checked(self._runtime.viewer_command(
                "events", "--trace-id", str(trace_id), "--output-format", "json"
            ), echo=False)
            for event in json.loads(result.stdout)["events"]:
                payload = event.get("payload", {})
                if event.get("variant") == "enforcement" and all(
                    payload.get(key) == value for key, value in expected.items()
                ):
                    return
            time.sleep(self._config.drain_interval_seconds)
        raise AssertionError(f"persisted file enforcement evidence missing: {expected}")

    def _require_configuration(self) -> None:
        if self._variant != "default_complete":
            return
        config = tomllib.loads((self._config.work_dir / "actraild.conf").read_text())
        for domain, operation in (("file", "open"), ("file", "mkdir"), ("file", "rmdir"), ("command", "exec"), ("network", "connect")):
            if config["access_control"][domain][operation]["backend"] != "lsm":
                raise AssertionError(f"default complete selected a different backend for {domain}.{operation}")
        pid = int((self._config.work_dir / "run/actraild.pid").read_text().strip())
        for descriptor in (Path("/proc") / str(pid) / "fd").iterdir():
            try:
                target = os.readlink(descriptor)
            except FileNotFoundError:
                continue
            if "fanotify" in target:
                raise AssertionError("default complete created a fanotify instance")

    def _require_resource_storage(self) -> str:
        config = tomllib.loads((self._config.work_dir / "actraild.conf").read_text())
        resource_path = Path(config["resource_profiling"]["storage"]["database"]["sqlite"]["path"])
        if not resource_path.is_absolute():
            resource_path = self._config.work_dir / resource_path
        host, workload = storage_counts(resource_path)
        assert self._runtime is not None
        result = self._runtime.run_checked(self._runtime.viewer_command(
            "host-resources", "--limit", "10000", "--output-format", "json",
        ), echo=False)
        assert_host_columns(resource_path, json.loads(result.stdout))
        with closing(sqlite3.connect(resource_path.as_uri() + "?mode=ro", uri=True)) as connection:
            if self._variant == "fanotify_shared_storage":
                if resource_path != Path(config["storage"]["sqlite"]["path"]):
                    raise AssertionError("shared resource storage path differs from main storage")
                if connection.execute("SELECT count(*) FROM events").fetchone()[0] == 0:
                    raise AssertionError("shared storage received no trace events")
        shared = "; shared main trace events preserved" if self._variant == "fanotify_shared_storage" else ""
        return f"host={host}, workload={workload}; column schema, references and host readback verified{shared}"

    def _patches(self) -> list[Path]:
        patches = [self._patch]
        if self._variant != "default_complete":
            patches.append(Path(__file__).parent / "fanotify" / "patch_config.toml")
        return patches

    def _write_patch(self) -> None:
        builder = (
            PatchBuilder().with_work_dir(self._config.work_dir).with_runtime_timing()
            .with_values({
                "storage": {"retention": {"enabled": False}},
                "enforcement": {"rules_path": str(self._rules)},
            })
        )
        if self._variant != "default_complete":
            shared = self._variant == "fanotify_shared_storage"
            resource_path = self._config.work_dir / (
                "shared.sqlite" if shared else f"resources-{time.time_ns()}.sqlite"
            )
            builder.with_values({
                "resource_profiling": {"storage": {"database": {"sqlite": {
                    "path": str(resource_path),
                }}}},
            })
            if shared:
                builder.with_values({"storage": {"sqlite": {"path": str(resource_path)}}})
        builder.build().save(self._patch)

    def cleanup(self, context: TestingContextSingleton) -> TestResult:
        del context
        failures = []
        try:
            if self._runtime is not None:
                result = self._runtime.stop()
                if result is not None and result.returncode != 0:
                    failures.append(result.output)
        except Exception as error:
            failures.append(str(error))
        finally:
            failure = self._provider.stop()
            if failure:
                failures.append(failure)
        return TestResult(
            TestStatus.FAILED if failures else TestStatus.PASSED,
            "; ".join(failures) if failures else "file-policy daemon and provider stopped",
        )
