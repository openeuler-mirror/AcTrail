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
from tests.v2.common.core import (
    CommonTestConfig,
    TestCase,
    TestCaseInputs,
    TestResult,
    TestStatus,
)
from tests.v2.common.runner import TestingContextSingleton
from tests.v2.regression.command_policy_xiaoo.provider import LocalXiaooProvider


class FilePolicyXiaooCase(TestCase):
    """Exercise default complete LSM control and explicit fanotify file control."""

    def __init__(self, inputs: TestCaseInputs):
        self._config = CommonTestConfig.from_environment(inputs, "FILE_POLICY_XIAOO")
        executable = os.environ.get("FILE_POLICY_XIAOO_E2E_BINARY") or shutil.which("xiaoo")
        if executable is None:
            raise RuntimeError("real Xiaoo executable is not available")
        self._xiaoo = Path(executable).resolve()
        self._timeout = float(os.environ.get("FILE_POLICY_XIAOO_E2E_READY_TIMEOUT_SECONDS", "15"))
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
        try:
            self._marker.write_text("", encoding="utf-8")
            provider_url = self._provider.start()
            self._xiaoo_config.write_text(
                '[llm]\nprovider = "deepseek"\nmodel = "deepseek-chat"\n'
                'api_key_env = "ACTRAIL_XIAOO_COMMAND_POLICY_KEY"\n'
                f"api_base = {json.dumps(provider_url)}\n"
                'max_tokens = 128\ncontext_window = 32768\nreasoning_effort = "off"\n',
                encoding="utf-8",
            )
            self._runtime = ActrailRuntime(
                self._config.repo,
                self._config.bin_dir,
                self._config.command_timeout_seconds,
                context.output,
                self._config.work_dir / "actraild.conf",
                self._patch,
            )
            for variant in ("default_complete", "fanotify", "fanotify_shared_storage"):
                self._variant = variant
                self._write_patch()
                for decision in ("deny", "allow"):
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
                    self._require_resource_storage()
                    results[f"{variant}_{decision}"] = TestResult(
                        TestStatus.PASSED,
                        f"{variant}: real Xiaoo trace-{trace_id} file open {decision} and selected backend audit verified",
                    )
            return TestResult(TestStatus.COMPOSITE, "real Xiaoo file policy", results)
        except Exception as error:
            results["failure"] = TestResult(TestStatus.FAILED, str(error))
            return TestResult(TestStatus.COMPOSITE, "real Xiaoo file policy", results)

    def _run_xiaoo(self, decision: str) -> int:
        assert self._runtime is not None
        environment = os.environ.copy()
        for name in ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"):
            environment.pop(name, None)
        environment.update(
            NO_PROXY="127.0.0.1,localhost",
            no_proxy="127.0.0.1,localhost",
            ACTRAIL_XIAOO_COMMAND_POLICY_KEY="local-test-key",
            PATH="/usr/bin",
        )
        result = self._runtime.run(
            self._runtime.control_command(
                "launch", "--seccomp-notify", "auto" if self._variant == "default_complete" else "disabled",
                "--name", f"v2-file-policy-xiaoo-{decision}", "--",
                self._xiaoo, "--cli", "run", "--config", self._xiaoo_config,
                "--tools", "bash", "--max-turns", "3", "--debug",
                "--prompt", "Use the Bash tool exactly once, then report its operating-system result.",
            ),
            timeout_seconds=self._config.launch_timeout_seconds,
            environment=environment,
        )
        if result.returncode != 0:
            raise AssertionError(f"Xiaoo failed: {result.output[-4000:]}")
        content = self._marker.read_text(encoding="utf-8")
        permission_error = any(
            message in result.output.lower()
            for message in ("permission denied", "operation not permitted")
        )
        if decision == "deny" and (content or not permission_error):
            raise AssertionError(f"denied open was not enforced: {result.output[-4000:]}")
        if decision == "allow" and (content != "ACTRAIL_XIAOO_COMMAND_OK" or permission_error):
            raise AssertionError(f"allowed open did not write its marker: {result.output[-4000:]}")
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

    def _require_resource_storage(self) -> None:
        config = tomllib.loads((self._config.work_dir / "actraild.conf").read_text())
        resource_path = Path(config["resource_profiling"]["storage"]["database"]["sqlite"]["path"])
        if not resource_path.is_absolute():
            resource_path = self._config.work_dir / resource_path
        with closing(sqlite3.connect(resource_path.as_uri() + "?mode=ro", uri=True)) as connection:
            if connection.execute("SELECT count(*) FROM resource_samples").fetchone()[0] == 0:
                raise AssertionError("resource storage received no real samples")
            if self._variant == "fanotify_shared_storage":
                if resource_path != Path(config["storage"]["sqlite"]["path"]):
                    raise AssertionError("shared resource storage path differs from main storage")
                if connection.execute("SELECT count(*) FROM events").fetchone()[0] == 0:
                    raise AssertionError("shared storage received no trace events")

    def _write_patch(self) -> None:
        if self._variant == "default_complete":
            ActrailRuntime.write_isolated_operator_config_patch(self._patch, self._config.work_dir)
            with self._patch.open("a", encoding="utf-8") as patch:
                patch.write(f'\n[enforcement]\nrules_path = {json.dumps(str(self._rules))}\n')
            return
        ActrailRuntime.write_isolated_operator_config_patch(
            self._patch,
            self._config.work_dir,
            payload_tls_enabled=False,
            payload_stdio_enabled=False,
        )
        with self._patch.open("a", encoding="utf-8") as patch:
            patch.write(
                '\n[payload.socket]\nenabled = false\n'
                '\n[capture]\nprofile_name = "file-policy-xiaoo-regression"\n'
                'capabilities = ["proc-lifecycle", "enforcement-file-permission-fanotify"]\n'
                'opportunistic_capabilities = []\n'
                '\n[ebpf]\nenabled = true\n'
                '\n[seccomp_notify]\nenabled = false\n'
                '\n[process_seccomp]\nenabled = false\n'
                '\n[command_control]\nenabled = false\n'
                '\n[network_control]\nenabled = false\n'
                '\n[access_control.file.open]\nbackend = "fanotify"\n'
                '\n[enforcement]\nenabled = true\nscope = "trace"\n'
                f'rules_path = {json.dumps(str(self._rules))}\n'
                'default_decision = "allow"\nmark_strategy = "parent-directories"\n'
                'audit_enabled = true\n'
                '\n[plugins.startup]\nenabled = false\n'
            )
            # Both layouts live beside the governed marker. A fresh resource file
            # exercises opening SQLite before permission interception starts.
            if self._variant == "fanotify_shared_storage":
                resource_path = self._config.work_dir / "shared.sqlite"
            else:
                resource_path = self._config.work_dir / f"resources-{time.time_ns()}.sqlite"
            patch.write(
                '\n[resource_profiling.storage.database.sqlite]\n'
                f'path = {json.dumps(str(resource_path))}\n'
            )
        if self._variant == "fanotify_shared_storage":
            contents = self._patch.read_text(encoding="utf-8")
            contents = re.sub(r'(\[storage\.sqlite\]\npath = )[^\n]+',
                              lambda match: match[1] + json.dumps(str(resource_path)), contents)
            self._patch.write_text(contents, encoding="utf-8")

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
