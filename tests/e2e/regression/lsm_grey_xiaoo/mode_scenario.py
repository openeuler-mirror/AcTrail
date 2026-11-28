#!/usr/bin/env python3
"""Check module-free startup modes with real Xiaoo and freshly generated defaults.

Uses existing release binaries without installing packages or changing kernel modules.
Requires the grey module to be absent from both the kernel and modprobe search path.
"""
import errno
import json
import os
from pathlib import Path
import shlex
import shutil
import subprocess
import sys

REPO = Path(__file__).resolve().parents[4]
sys.path.insert(0, str(REPO))
from tests.e2e.common.actrail_runtime import ActrailRuntime
from tests.e2e.common.core import TestOutput
from tests.e2e.common.patch_builder import PatchBuilder
from tests.e2e.regression.lsm_access_control_xiaoo.services import WorkloadProvider


class ModeScenario:
    def __init__(self, work, backend, mode, output, bin_dir=None):
        self.bin_dir = bin_dir or REPO / "target/release"
        self.work, self.backend, self.mode, self.output = work, backend, mode, output
        self.provider = None
        self.runtime = None

    def run(self):
        xiaoo = Path(os.environ.get("LSM_GREY_XIAOO_E2E_BINARY") or shutil.which("xiaoo")).resolve()
        protected = self.work / "protected"
        protected.mkdir()
        for name in ("allow", "deny", "grey", "read-deny", "read-allow"):
            (protected / name).write_text("PAYLOAD\n")
        rules = [f"allow allow open {protected}/allow", f"deny deny open {protected}/deny"]
        if self.backend == "lsm":
            for name, operation, fallback in (("grey", "open", "deny"), ("read-deny", "read", "deny"), ("read-allow", "read", "allow")):
                rules.append(f"{name} gray sync-plugin absent.plugin timeout-ms 1000 concurrency 1 fallback {fallback} {operation} {protected}/{name}")
        (self.work / "file.rules").write_text("\n".join(rules) + "\n")
        for domain in ("command", "network"):
            (self.work / f"{domain}.rules").write_text("")
        configuration = REPO / "tests/e2e/regression/lsm_access_control_xiaoo/configuration"
        patch = (PatchBuilder().apply(configuration / "patch_config.toml")
                 .apply(configuration / self.backend / "patch_config.toml")
                 .apply(Path(__file__).parent / "configuration" / f"{self.mode}.toml")
                 .with_work_dir(self.work).with_runtime_timing()
                 .with_values({"enforcement": {"rules_path": str(self.work / "file.rules")},
                               "command_control": {"rules_path": str(self.work / "command.rules")},
                               "network_control": {"rules_path": str(self.work / "network.rules")}})
                 .build().save(self.work / "patch.toml"))
        self.runtime = ActrailRuntime(REPO, self.bin_dir, 60, self.output,
                                      self.work / "operator.conf", operator_config_patch=patch)
        if self.mode == "required" and self.backend == "lsm":
            try:
                self.runtime.prepare()
            except RuntimeError:
                log = (self.work / "log/actraild.log").read_text()
                assert "lsm_preflight" in log and "lsm_attach" in log, log
                return
            raise AssertionError("required mode started without the module")
        self.runtime.prepare()
        log = (self.work / "log/actraild.log").read_text()
        if self.backend == "lsm" and self.mode == "auto":
            assert "LSM grey unavailable" in log, log
        else:
            assert "LSM grey unavailable" not in log and "LSM grey dynamic" not in log, log
        workload = shlex.join([sys.executable, str(Path(__file__).with_name("workload.py")),
                              "--work", str(self.work), "--xiaoo", str(xiaoo)])
        self.provider = WorkloadProvider(REPO, self.work, workload, 15)
        url = self.provider.start()
        (self.work / "xiaoo.toml").write_text(
            '[llm]\nprovider="deepseek"\nmodel="deepseek-chat"\napi_key_env="ACTRAIL_XIAOO_LSM_KEY"\n'
            f'api_base="{url}"\nmax_tokens=128\ncontext_window=32768\nreasoning_effort="off"\n')
        (self.work / "phase").write_text("normal")
        environment = os.environ.copy()
        for name in ("ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY", "all_proxy", "https_proxy", "http_proxy"):
            environment.pop(name, None)
        environment.update(ACTRAIL_XIAOO_LSM_KEY="local-test-key", NO_PROXY="127.0.0.1,localhost", PATH="/usr/bin")
        result = self.runtime.run(self.runtime.control_command(
            "launch", "--seccomp-notify", "disabled" if self.backend == "lsm" else "required",
            "--", xiaoo, "--cli", "run", "--config", self.work / "xiaoo.toml",
            "--tools", "bash", "--max-turns", "3", "--prompt", "Use the Bash tool once."),
            timeout_seconds=90, environment=environment)
        (self.work / "launch.log").write_text(result.output)
        assert result.returncode == 0, result.output
        actual = json.loads((self.work / "results-normal.json").read_text())
        denied = errno.EACCES if self.backend == "lsm" else errno.EPERM
        expected = [0, denied] + ([errno.EACCES] * 5 + [0] if self.backend == "lsm" else [0] * 6)
        assert [item["errno"] for item in actual["results"]] == expected, actual

    def close(self):
        if self.provider:
            self.provider.stop()
        if self.runtime:
            self.runtime.stop()

