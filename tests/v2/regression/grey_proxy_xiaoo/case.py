"""Verify context-enhanced grey decisions with the real Xiaoo and managed proxy."""
import errno
import json
import os
import shlex
import shutil
import signal
import socket
import subprocess
import sys
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path

from tests.v2.common.patch_builder import PatchBuilder
from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.plugin_test_environment import PluginRuntimeSpec, PluginTestEnvironment
from tests.v2.regression.lsm_access_control_xiaoo.services import WorkloadProvider
from tests.v2.regression.lsm_grey_xiaoo.case import LsmGreyXiaooCase


class GreyProxyXiaooCase(LsmGreyXiaooCase):
    def _run(self, context):
        self.output = context.output
        self.requests = []
        records = self.requests
        sensitive = str(self.work / "protected/grey")
        self.target_socket = socket.socket()
        self.target_socket.bind(("127.0.0.1", 0))
        self.target_socket.listen()
        self.target_port = self.target_socket.getsockname()[1]
        class Handler(BaseHTTPRequestHandler):
            def do_PUT(self):
                body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
                records.append(body)
                actions = body.get("history", {}).get("actions", [])
                read_allowed = any(a.get("operation") == "read" and a.get("target") == sensitive and a.get("decision") == "allow" for a in actions)
                has_args = any(a.get("operation") == "exec" and "--send" in a.get("command", "") and "workload.py" in a["command"] for a in actions)
                verdict = "deny" if body["operation"] == "connect" and read_allowed and has_args else "allow"
                response = json.dumps({"verdict": verdict, "reason": "history-aware test"}).encode()
                self.send_response(200)
                self.send_header("Content-Length", str(len(response)))
                self.end_headers()
                self.wfile.write(response)
            def log_message(self, *_):
                pass
        self.http = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        threading.Thread(target=self.http.serve_forever, daemon=True).start()
        self.service_url = f"http://127.0.0.1:{self.http.server_port}"
        self._prepare_proxy()
        self.environment = PluginTestEnvironment(
            self.config, self.output, operator_config=self.work / "actraild.conf",
            operator_config_patches=[self._patch()], web_host="127.0.0.1", web_port=0,
            plugin=PluginRuntimeSpec(package="grey-forwarding", instance_id="wasm.security-service",
                plugin_id="wasm.grey-forwarding", runtime="wasm", load_grants={"http_request": True}),
        )
        self.environment.prepare()
        children = self._proxy_pids()
        if len(children) != 1:
            raise AssertionError(f"expected one proxy: {children}")
        command = shlex.join([sys.executable, str(Path(__file__).with_name("workload.py")), str(self.work), "--read", str(self.target_port)])
        self.provider = WorkloadProvider(self.config.repo, self.work, command, 15)
        provider_url = self.provider.start()
        (self.work / "xiaoo.toml").write_text(
            '[llm]\nprovider="deepseek"\nmodel="deepseek-chat"\napi_key_env="ACTRAIL_XIAOO_LSM_KEY"\n'
            f'api_base="{provider_url}"\nmax_tokens=128\ncontext_window=32768\nreasoning_effort="off"\n')
        result = self._launch("context")
        if [item["errno"] for item in result["results"]] != [0, errno.EACCES]:
            raise AssertionError(f"grey decisions failed: {result}")
        if len(records) != 2:
            raise AssertionError(f"expected two external decisions: {records}")
        outbound = records[-1]
        actions = outbound["history"]["actions"]
        if outbound["operation"] != "connect" or not any(a.get("operation") == "read" and a.get("target") == sensitive and a.get("decision") == "allow" for a in actions):
            raise AssertionError(f"outbound decision lacks sensitive read: {outbound}")
        if not any("--send" in a.get("command", "") for a in actions):
            raise AssertionError(f"execution parameters missing: {actions}")
        self._verify_compact(records)
        # A one-record window must retain the latest exec, not the oldest shell event.
        config = self.environment.current_config()
        config["grey_proxy"]["max_actions"] = 1
        self.environment.update_config(config)
        children = self._proxy_pids()
        limited = self._launch("limited")
        latest = records[-1]["history"]
        if [item["errno"] for item in limited["results"]] != [0, 0] or len(latest["actions"]) != 1 or "--send" not in latest["actions"][0].get("command", "") or not latest["truncated"]:
            raise AssertionError(f"latest-record selection failed: {latest}, {limited}")
        self._verify_compact(records)
        (self.work / "proxy-requests.json").write_text(json.dumps(records, indent=2))
        self.http.shutdown()
        self.http.server_close()
        self.http = None
        stopped = self._launch("stopped")
        if [item["errno"] for item in stopped["results"]] != [0, errno.EACCES]:
            raise AssertionError(f"external failure did not use grey fallback: {stopped}")
        self.environment.unload_plugin()
        self._await_exit(children)
        api = self.environment.api
        api.load(api.package_key("wasm.grey-forwarding"), "wasm.security-service", {"http_request": True})
        children = self._proxy_pids()
        if len(children) != 1:
            raise AssertionError(f"reloaded proxy missing: {children}")
        self.environment.runtime.stop()
        self._await_exit(children)
        # Start the same configured daemon again to exercise abrupt owner loss.
        runtime = self.environment.runtime
        runtime.run_checked([*runtime._daemon_command(), "start"])
        runtime._started = True
        api.load(api.package_key("wasm.grey-forwarding"), "wasm.security-service", {"http_request": True})
        children = self._proxy_pids()
        if len(children) != 1:
            raise AssertionError(f"proxy missing before owner termination: {children}")
        child = next(iter(children))
        status = Path(f"/proc/{child}/status").read_text()
        parent = int(next(line.split()[1] for line in status.splitlines() if line.startswith("PPid:")))
        os.kill(parent, signal.SIGKILL)
        self._await_exit(children)
        runtime.stop()
        self.environment._daemon_started = False
        return TestResult(TestStatus.PASSED, "real Xiaoo verified sensitive-read history, execution arguments, latest-record selection, context-based outbound denial and child cleanup")

    def _verify_compact(self, records):
        for request in records:
            if set(request) != {"decision_id", "operation", "target", "history"}:
                raise AssertionError(f"unexpected external fields: {request.keys()}")
            for action in request["history"]["actions"]:
                if set(action) - {"time", "operation", "target", "decision", "protocol", "command", "result", "parameters_truncated", "parameters_unavailable"}:
                    raise AssertionError(f"unexpected history fields: {action}")

    def _patch(self):
        base = super()._patch()
        return PatchBuilder().apply(base).with_values({"capture": {"capabilities": [
            "proc-lifecycle", "proc-exec-context", "net-transport", "socket-plaintext-payload",
            "enforcement-file-permission-lsm", "enforcement-command-execution-lsm", "enforcement-network-connect-lsm"
        ]}}).build().save(self.work / "proxy-patch.toml")

    def _prepare_proxy(self):
        (self.work / "protected").mkdir()
        (self.work / "protected/grey").write_text("GREY_PROXY_CONTENT\n")
        (self.work / "file.rules").write_text(f"grey gray sync-plugin wasm.security-service timeout-ms 1000 concurrency 1 fallback allow read {self.work}/protected/grey\n")
        (self.work / "command.rules").write_text("")
        (self.work / "network.rules").write_text(f"sensitive-upload sync-plugin wasm.security-service timeout-ms 1000 concurrency 1 fallback deny connect 127.0.0.1:{self.target_port}\n")
        source = self.config.repo / "crates/plugins/grey_forwarding"
        target = Path(os.environ.get("CARGO_TARGET_DIR", self.config.repo / "target"))
        subprocess.run(["cargo", "build", "--release", "--locked", "--target", "wasm32-wasip2", "--target-dir", str(target),
            "--manifest-path", str(source / "Cargo.toml")], check=True)
        package = self.work / "plugins/grey-forwarding"
        package.mkdir(parents=True)
        shutil.copy2(target / "wasm32-wasip2/release/actrail_grey_forwarding.wasm", package / "component.wasm")
        endpoint = self.service_url + "/analyze"
        (package / "grey-forwarding.config.json").write_text(json.dumps({"grey_proxy": {
            "endpoint": endpoint, "window_ms": 60000, "max_actions": 64}}))
        (package / "config.schema.json").write_text(json.dumps({"type": "object", "properties": {"grey_proxy": {"type": "object"}}, "required": ["grey_proxy"], "additionalProperties": False}))
        (package / "grey-forwarding.plugin.toml").write_text(
            '[general]\nid="wasm.grey-forwarding"\napi_version="actrail.plugin.v2"\nrole="control-decider"\nruntime="wasm"\n'
            '[host]\ncapabilities=["http.request","command-execution.current-context-query"]\n'
            f'[host.http]\nallowed_urls=["{endpoint}"]\ntimeout_ms=1000\nrequest_max_bytes=65536\nresponse_max_bytes=65536\n'
            '[runtime.wasm]\nartifact_path="component.wasm"\nabi="wit-component"\n'
            '[runtime.wasm.resources]\nfuel_per_call=10000000\nmemory_max_bytes=8388608\n'
            '[role.control-decider.resources]\nconcurrency_limit=1\n'
            '[plugin_config]\nformat="json"\nschema_ref="config.schema.json"\nrequired=true\n')

    def _proxy_pids(self):
        pids = set()
        for root in Path("/proc").iterdir():
            if root.name.isdigit():
                try:
                    if (root / "exe").resolve().name == "actraild-grey-proxy":
                        status = (root / "status").read_text()
                        parent = next(line.split()[1] for line in status.splitlines() if line.startswith("PPid:"))
                        if str(self.work / "actraild.conf").encode() not in Path(f"/proc/{parent}/cmdline").read_bytes():
                            continue
                        pids.add(int(root.name))
                except OSError:
                    pass
        return pids

    def _await_exit(self, pids):
        deadline = time.monotonic() + 5
        while time.monotonic() < deadline:
            if all(not Path(f"/proc/{pid}/exe").exists() for pid in pids):
                return
            time.sleep(0.05)
        raise AssertionError(f"proxy did not exit: {pids}")

    def cleanup(self, context):
        if getattr(self, "target_socket", None):
            self.target_socket.close()
        if getattr(self, "http", None):
            self.http.shutdown()
            self.http.server_close()
        return super().cleanup(context)
