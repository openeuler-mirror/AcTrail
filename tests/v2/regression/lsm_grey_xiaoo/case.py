"""Exercise kernel fast decisions and sleeping LSM authorization using real Xiaoo."""
from __future__ import annotations

import errno
import json
import os
import re
import select
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import traceback
from pathlib import Path
from urllib.request import Request, urlopen

from tests.v2.common.core import CommonTestConfig, TestCase, TestCaseInputs, TestResult, TestStatus
from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.plugin_test_environment import PluginRuntimeSpec, PluginTestEnvironment
from tests.v2.regression.lsm_access_control_xiaoo.services import WorkloadProvider
from tests.v2.regression.lsm_grey_xiaoo.argv_case import ArgvCase


class LsmGreyXiaooCase(TestCase):
    def __init__(self, inputs: TestCaseInputs):
        self.config = CommonTestConfig.from_environment(inputs, "LSM_GREY_XIAOO")
        executable = os.environ.get("LSM_GREY_XIAOO_E2E_BINARY") or shutil.which("xiaoo")
        if executable is None:
            raise RuntimeError("set LSM_GREY_XIAOO_E2E_BINARY to the real Xiaoo executable")
        self.xiaoo = Path(executable).resolve()
        self.work = self.config.work_dir
        self.environment = None
        self.provider = None
        self.service = None
        self.service_log = None

    def run(self, context) -> TestResult:
        try:
            return self._run(context)
        except Exception:
            (self.work / "failure.log").write_text(traceback.format_exc())
            raise
        finally:
            self._preserve_evidence()

    def _run(self, context) -> TestResult:
        self.output = context.output
        self._start_service()
        self._prepare_files()
        patch = self._patch()
        self.environment = PluginTestEnvironment(
            self.config, self.output, operator_config=self.work / "actraild.conf",
            operator_config_patches=[patch], web_host="127.0.0.1", web_port=0,
            plugin=PluginRuntimeSpec(package="security-service", instance_id="wasm.security-service",
                                     plugin_id="wasm.security-service", runtime="wasm",
                                     load_grants={"http_request": True, "command_policy_rules_apply": [
                                         {"decision": "deny", "path_scope": str(self.curl)},
                                         {"decision": "allow", "path_scope": str(self.cat)},
                                     ]}),
        )
        self.environment.prepare()
        command = shlex.join([sys.executable, str(Path(__file__).with_name("workload.py")),
                              "--work", str(self.work), "--xiaoo", str(self.xiaoo)])
        self.provider = WorkloadProvider(self.config.repo, self.work, command, 15)
        provider_url = self.provider.start()
        (self.work / "xiaoo.toml").write_text(
            '[llm]\nprovider="deepseek"\nmodel="deepseek-chat"\n'
            'api_key_env="ACTRAIL_XIAOO_LSM_KEY"\n'
            f'api_base="{provider_url}"\nmax_tokens=128\ncontext_window=32768\nreasoning_effort="off"\n')
        normal = self._launch("normal")
        expected = [0, errno.EACCES, 0, errno.EACCES, 0, errno.EACCES, errno.EACCES, 0]
        if [item["errno"] for item in normal["results"]] != expected:
            raise AssertionError(f"unexpected syscall results: {normal}")
        if not normal["results"][6].get("opened"):
            raise AssertionError("non-sleepable fallback must deny read after successful open")
        requests = self._requests()
        if len(requests) != 6 or any(item["pid"] != normal["pid"] for item in requests):
            raise AssertionError(f"expected exactly four open and two open-time read requests from workload PID: {requests}")
        if any(item["trace_id"] != str(normal["trace_id"]) for item in requests):
            raise AssertionError("service requests are not correlated to the launched trace")
        if [item["operation"] for item in requests] != ["open"] * 4 + ["read"] * 2:
            raise AssertionError(f"unexpected service operation sequence: {requests}")
        if len({item["decision_id"] for item in requests}) != 6:
            raise AssertionError("each access must carry a distinct decision ID")
        if [item["verdict"] for item in requests[:4]] != ["allow", "deny", "allow", "deny"]:
            raise AssertionError("security service did not alternate decisions")
        commands = self._launch("commands")
        get, post = commands["results"]
        if get["errno"] != 0 or get.get("returncode") != 0 or get.get("stdout") != "CURL_TARGET_OK" or post["errno"] != errno.EACCES:
            raise AssertionError(f"curl GET/POST decisions failed: {commands}")
        command_requests = self._requests()[6:]
        if len(command_requests) != 2 or [item["verdict"] for item in command_requests] != ["allow", "deny"]:
            raise AssertionError(f"expected two command decisions: {command_requests}")
        for item, result in zip(command_requests, commands["results"]):
            if item["operation"] != "exec" or item["argv"] != result["argv"] or item["executable"] != str(self.curl):
                raise AssertionError(f"incorrect command context: {item}")
            if item["trace_id"] != str(commands["trace_id"]) or item["generation"] <= 0:
                raise AssertionError(f"incorrect command actor identity: {item}")
        if command_requests[0]["pid"] != get["pid"]:
            raise AssertionError("command context PID does not identify the curl child")
        if json.loads((self.work / "target-requests.json").read_text()) != ["GET"]:
            raise AssertionError("denied POST reached the HTTP target")
        argv = self._launch("argv")
        ArgvCase(self.work).verify(argv, self._requests()[8:])
        self._verify_instance_collision()
        self._verify_management_deadline()
        before_reuse = len(self._requests())
        with urlopen(Request(self.service_url + "/reuse", data=b'{"enabled": true}',
                             headers={"Content-Type": "application/json"}), timeout=5) as response:
            response.read()
        reused = self._launch("reuse")
        self._verify_reuse(reused, first=True)
        learned = self._requests()[before_reuse:]
        if len(learned) != 2 or [(item["executable"], item["verdict"], item.get("reuse")) for item in learned] != [
                (str(self.curl), "deny", "executable"), (str(self.cat), "allow", "executable")]:
            raise AssertionError(f"expected exactly one external decision per executable: {learned}")
        if any(item["trace_id"] != str(reused["trace_id"]) for item in learned):
            raise AssertionError("reusable decisions belong to another trace")
        request_count = len(self._requests())
        # Slow and unavailable services must produce the configured local deny fallback.
        with urlopen(Request(self.service_url + "/delay", data=b'{"seconds": 2}',
                             headers={"Content-Type": "application/json"}), timeout=5) as response:
            response.read()
        timeout = self._launch("timeout")
        if timeout["results"][0]["errno"] != errno.EACCES or len(self._requests()) != request_count + 1:
            raise AssertionError(f"service timeout did not fail locally: {timeout}")
        self._stop_service()
        stopped = self._launch("stopped")
        if stopped["results"][0]["errno"] != errno.EACCES or len(self._requests()) != request_count + 1:
            raise AssertionError(f"stopped service did not fail locally: {stopped}")
        self._verify_reuse(self._launch("reuse-stopped"), first=False)
        if len(self._requests()) != request_count + 1:
            raise AssertionError("kernel executable rules contacted the stopped service")
        return TestResult(TestStatus.PASSED,
                          "real Xiaoo verified grey-to-kernel curl deny/cat allow reuse, kernel argv, file decisions and service failures")

    def _verify_management_deadline(self) -> None:
        api = self.environment.api
        evidence = []
        for mode in ("slow-headers", "slow-body"):
            started = time.monotonic()
            try:
                api.command("wasm.security-service", [self.service_url + "/management", mode])
            except RuntimeError as error:
                elapsed = time.monotonic() - started
                message = str(error)
                if elapsed >= 1.5 or not any(word in message.lower() for word in ("timeout", "timed out", "deadline")):
                    raise AssertionError(f"management HTTP exceeded its budget or failed unexpectedly ({elapsed}s): {message}")
                evidence.append({"mode": mode, "elapsed": elapsed, "error": message})
            else:
                raise AssertionError(f"slow management HTTP succeeded: {mode}")
            response = api.command("wasm.security-service", [self.service_url + "/management", "fast"])
            evidence.append({"mode": "fast-same-instance", "response": response})
            if response.get("command", {}).get("exit_code") != 0 or response["command"].get("stdout") != "MANAGEMENT_HTTP_OK":
                raise AssertionError(f"management command failed on the same instance: {response}")
        # This package reserves the full command budget, so even a fast URL must
        # be rejected before the service receives it.
        api.load(api.package_key("wasm.security-numeric"), "http-budget", self.environment.plugin.load_grants)
        try:
            try:
                api.command("http-budget", [self.service_url + "/management", "fast"])
            except RuntimeError as error:
                if "finish reserve" not in str(error):
                    raise
                evidence.append({"mode": "insufficient-budget", "error": str(error)})
            else:
                raise AssertionError("HTTP started without enough time for the finish reserve")
        finally:
            api.unload("http-budget")
        (self.work / "management-deadlines.json").write_text(json.dumps(evidence, indent=2))
        if json.loads((self.work / "management-requests.json").read_text()) != ["slow-headers", "fast", "slow-body", "fast"]:
            raise AssertionError("management commands did not reach the real HTTP service")

    def _verify_instance_collision(self) -> None:
        # The primary plugin is registration 1; a second plugin is named "1".
        api = self.environment.api
        api.load(api.package_key("wasm.security-numeric"), "1", self.environment.plugin.load_grants)
        try:
            for swapped in (False, True):
                with urlopen(Request(self.service_url + "/collision", data=json.dumps({"swapped": swapped}).encode(),
                                     headers={"Content-Type": "application/json"}), timeout=5) as response:
                    response.read()
                before = len(self._requests())
                document = self._launch("collision-swapped" if swapped else "collision")
                expected = [errno.EACCES, 0] if swapped else [0, errno.EACCES]
                if [item["errno"] for item in document["results"]] != expected:
                    raise AssertionError(f"registration collision changed authorization: {document}")
                requests = self._requests()[before:]
                if len(requests) != 2 or [item["endpoint"] for item in requests] != ["/analyze", "/analyze/numeric"]:
                    raise AssertionError(f"decision reached the wrong plugin endpoint: {requests}")
                if any(item["pid"] != document["pid"] or item["trace_id"] != str(document["trace_id"]) for item in requests):
                    raise AssertionError(f"collision decisions belong to another actor: {requests}")
        finally:
            api.unload("1")

    def _verify_reuse(self, document: dict, first: bool) -> None:
        expected_count = self.reuse_rounds * 2
        if len(document["results"]) != expected_count:
            raise AssertionError(f"missing repeated executable calls: {document}")
        for index, result in enumerate(document["results"]):
            if index % 2 == 0:
                if result["name"] != "curl" or result["errno"] != errno.EACCES:
                    raise AssertionError(f"reused curl deny failed: {result}")
            elif result["name"] != "cat" or result["errno"] or result.get("returncode") != 0 or result.get("stdout") != "REUSABLE_CAT_CONTENT\n":
                raise AssertionError(f"reused cat allow failed: {result}")
        runtime = self.environment.runtime
        expected = {}
        for name, executable, decision in (("curl", self.curl, "deny"), ("cat", self.cat, "allow")):
            for rule_id, count in ((f"{name}-grey", int(first)),
                                   (f"reuse:{executable}", self.reuse_rounds - int(first))):
                expected[(rule_id, decision)] = count
        # LSM audit draining and storage are asynchronous after launch returns.
        deadline = time.monotonic() + self.config.launch_timeout_seconds
        while True:
            events = runtime.run_checked(runtime.viewer_command(
                "events", "--trace-id", str(document["trace_id"]), "--output-format", "json"), echo=False)
            evidence = json.loads(events.stdout)
            payloads = [event.get("payload", {}) for event in evidence["events"]]
            counts = {key: sum(p.get("rule_id") == key[0] and p.get("decision") == key[1]
                               for p in payloads) for key in expected}
            if counts == expected or time.monotonic() >= deadline:
                break
            time.sleep(self.config.drain_interval_seconds)
        (self.work / f'reuse-audit-{document["trace_id"]}.json').write_text(json.dumps(evidence, indent=2))
        if counts != expected:
            raise AssertionError(f"reusable rule audit counts differ: expected {expected}, got {counts}")

    def _preserve_evidence(self) -> None:
        # The shared runner deletes both its workspace and log directory during cleanup.
        root = Path(os.environ.get("LSM_GREY_XIAOO_E2E_ARTIFACT_DIR",
                                   self.config.repo / "tmp/lsm-grey-evidence")).resolve()
        root.mkdir(parents=True, exist_ok=True)
        destination = Path(tempfile.mkdtemp(prefix="run-", dir=root))
        for pattern in ("*.log", "*.json", "*.rules", "patch.toml", "log/*.log"):
            for source in self.work.glob(pattern):
                if source.is_file():
                    target = destination / source.relative_to(self.work)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, target)
        print(f"LSM grey evidence: {destination}", flush=True)

    def _start_service(self) -> None:
        self.service_log = (self.work / "security-service.log").open("w")
        self.service = subprocess.Popen(
            [sys.executable, str(Path(__file__).with_name("security_service.py")),
             "--state", str(self.work / "security-requests.json")],
            stdout=subprocess.PIPE, stderr=self.service_log, text=True)
        ready, _, _ = select.select([self.service.stdout], [], [], 15)
        if not ready:
            raise RuntimeError("security service did not become ready")
        self.service_url = self.service.stdout.readline().strip()
        if not self.service_url.startswith("http://127.0.0.1:"):
            raise RuntimeError("security service exited before readiness")

    def _prepare_files(self) -> None:
        protected = self.work / "protected"
        protected.mkdir()
        for name in ("allow", "deny", "grey", "read-deny", "read-allow", "collision-primary", "collision-numeric"):
            (protected / name).write_text("SENSITIVE_PAYLOAD\n")
        rules = [f"static-allow allow open {protected}/allow", f"static-deny deny open {protected}/deny"]
        for name, operation, fallback in (("grey", "open", "deny"), ("read-deny", "read", "deny"), ("read-allow", "read", "allow")):
            rules.append(f"{name} gray sync-plugin wasm.security-service timeout-ms 1000 concurrency 1 fallback {fallback} {operation} {protected}/{name}")
        for name, target in (("collision-primary", "wasm.security-service"), ("collision-numeric", "1")):
            rules.append(f"{name} gray sync-plugin {target} timeout-ms 1000 concurrency 1 fallback deny open {protected}/{name}")
        (self.work / "file.rules").write_text("\n".join(rules) + "\n")
        # The capture profile enables all LSM domains; empty rules preserve their allow defaults.
        for domain in ("command", "network"):
            (self.work / f"{domain}.rules").write_text("")
        curl = shutil.which("curl")
        if curl is None:
            raise RuntimeError("curl is required for the command grey case")
        self.curl = Path(curl).resolve()
        cat = shutil.which("cat")
        if cat is None:
            raise RuntimeError("cat is required for executable reuse")
        self.cat = Path(cat).resolve()
        self.reuse_rounds = int(os.environ.get("LSM_GREY_XIAOO_E2E_REUSE_ROUNDS", "3"))
        if self.reuse_rounds < 2:
            raise ValueError("LSM_GREY_XIAOO_E2E_REUSE_ROUNDS must be at least 2")
        (self.work / "reuse-content").write_text("REUSABLE_CAT_CONTENT\n")
        (self.work / "reuse-config.json").write_text(json.dumps({
            "curl": str(self.curl), "cat": str(self.cat), "rounds": self.reuse_rounds,
            "url": self.service_url + "/target", "file": str(self.work / "reuse-content"),
        }))
        (self.work / "command.rules").write_text(
            f"curl-grey gray sync-plugin wasm.security-service exec {self.curl} priority 0\n"
            + f"cat-grey gray sync-plugin wasm.security-service exec {self.cat} priority 0\n"
            + ArgvCase(self.work).prepare())
        (self.work / "curl-config.json").write_text(json.dumps({"executable": str(self.curl), "url": self.service_url + "/target"}))
        fixture = self.config.repo / "tests/plugins/wasm-security-service/fixture-src"
        target = Path(os.environ.get("CARGO_TARGET_DIR", self.config.repo / "target"))
        target = target if target.is_absolute() else self.config.repo / target
        subprocess.run(["cargo", "fmt", "--manifest-path", str(fixture / "Cargo.toml")], check=True)
        subprocess.run(["cargo", "build", "--release", "--locked", "--target", "wasm32-wasip2", "--target-dir", str(target),
                        "--manifest-path", str(fixture / "Cargo.toml")], check=True)
        package = self.work / "plugins/security-service"
        package.mkdir(parents=True)
        shutil.copy2(target / "wasm32-wasip2/release/actrail_component_security_service_fixture.wasm", package / "component.wasm")
        (package / "security-service.config.json").write_text(json.dumps({"endpoint": self.service_url + "/analyze"}))
        (package / "config.schema.json").write_text('{"type":"object","properties":{"endpoint":{"type":"string"}},"required":["endpoint"],"additionalProperties":false}\n')
        (package / "security-service.plugin.toml").write_text(
            '[general]\nid="wasm.security-service"\napi_version="actrail.plugin.v2"\nrole="control-decider"\nruntime="wasm"\n'
            '[host]\ncapabilities=["http.request","command-execution.current-context-query","command-policy.rules.read","command-policy.rules.apply"]\n'
            f'[host.http]\nallowed_urls=["{self.service_url}/analyze","{self.service_url}/analyze/numeric","{self.service_url}/management"]\ntimeout_ms=5000\nrequest_max_bytes=65536\nresponse_max_bytes=65536\n'
            '[hostcall_limits.plugin_command]\ntimeout_ms=300\n'
            '[runtime.wasm]\nartifact_path="component.wasm"\nabi="wit-component"\n'
            '[runtime.wasm.resources]\nfuel_per_call=1000000\nmemory_max_bytes=8388608\n'
            '[role.control-decider.resources]\nconcurrency_limit=1\n'
            '[plugin_config]\nformat="json"\nschema_ref="config.schema.json"\nrequired=true\n')
        numeric = self.work / "plugins/security-numeric"
        shutil.copytree(package, numeric)
        manifest = numeric / "security-service.plugin.toml"
        manifest.write_text(manifest.read_text().replace('id="wasm.security-service"', 'id="wasm.security-numeric"'))
        manifest.write_text(manifest.read_text().replace('[host.http]\n', '[host.http]\nfinish_reserve_ms=300\n'))
        (numeric / "security-service.config.json").write_text(json.dumps({"endpoint": self.service_url + "/analyze/numeric"}))

    def _patch(self) -> Path:
        configuration = self.config.repo / "tests/v2/regression/lsm_access_control_xiaoo/configuration"
        grey_configuration = Path(__file__).parent / "configuration"
        mode = os.environ.get("LSM_GREY_XIAOO_E2E_MODE", "required")
        return (PatchBuilder().apply(configuration / "patch_config.toml")
                .apply(configuration / "lsm/patch_config.toml")
                .apply(grey_configuration / "dynamic.toml")
                .apply(grey_configuration / f"{mode}.toml")
                .with_work_dir(self.work).with_runtime_timing()
                .with_values({"enforcement": {"rules_path": str(self.work / "file.rules")},
                    "command_control": {"rules_path": str(self.work / "command.rules")},
                    "network_control": {"rules_path": str(self.work / "network.rules")}})
                .build().save(self.work / "patch.toml"))

    def _launch(self, phase: str) -> dict:
        (self.work / "phase").write_text(phase)
        environment = os.environ.copy()
        for name in ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"):
            environment.pop(name, None)
        environment.update(NO_PROXY="127.0.0.1,localhost", no_proxy="127.0.0.1,localhost",
                           ACTRAIL_XIAOO_LSM_KEY="local-test-key", PATH="/usr/bin")
        runtime = self.environment.runtime
        result = runtime.run(runtime.control_command(
            "launch", "--seccomp-notify", "disabled", "--name", f"lsm-grey-{phase}", "--",
            self.xiaoo, "--cli", "run", "--config", self.work / "xiaoo.toml", "--tools", "bash",
            "--max-turns", "3", "--debug", "--prompt", "Use the Bash tool exactly once, then report its operating-system results."),
            timeout_seconds=self.config.launch_timeout_seconds, environment=environment)
        (self.work / f"launch-{phase}.log").write_text(result.output)
        match = re.search(r"trace trace-(\d+) entered Active", result.output)
        if result.returncode != 0 or match is None:
            raise AssertionError(f"real Xiaoo launch failed: {result.output[-4000:]}")
        document = json.loads((self.work / f"results-{phase}.json").read_text())
        document["trace_id"] = int(match.group(1))
        return document

    def _requests(self) -> list[dict]:
        return json.loads((self.work / "security-requests.json").read_text())

    def _stop_service(self) -> None:
        if self.service is not None:
            self.service.terminate()
            self.service.wait(timeout=5)
            self.service.stdout.close()
            self.service = None

    def cleanup(self, context) -> TestResult:
        self.output = context.output
        failures = []
        for service in (self.provider,):
            if service is not None:
                try:
                    service.stop()
                except Exception as error:
                    failures.append(str(error))
        self._stop_service()
        if self.service_log:
            self.service_log.close()
        if self.environment is not None:
            result = self.environment.cleanup()
            if result.has_failure():
                failures.append(str(result))
        return TestResult(TestStatus.FAILED if failures else TestStatus.PASSED,
                          "; ".join(failures) or "services stopped and traces cleaned")
