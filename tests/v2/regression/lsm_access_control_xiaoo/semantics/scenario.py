"""Publish operation-specific policies while a real Xiaoo retains file access."""
from concurrent.futures import ThreadPoolExecutor
import errno
import json
from pathlib import Path
import shlex
import sys
import time
import tomllib

from tests.v2.common.core import TestResult, TestStatus
from tests.v2.common.actrail_runtime import ActrailRuntime
from ..dynamic import DynamicPolicies
from .resources import FileResources


class FileSemanticsScenario:
    phases = ("empty", "open", "write", "read", "delete", "attr", "restore")

    def __init__(self, case):
        self.case = case
        self.root = case._work / "semantics"

    @staticmethod
    def command(case):
        return shlex.join([
            sys.executable, str(Path(__file__).with_name("workload.py")),
            "--root", str(case._work / "semantics"), "--xiaoo", str(case._xiaoo),
            "--timeout", str(case._config.launch_timeout_seconds), "--poll", str(case._poll),
            *(["--resources"] if case._selection.name == "lsm" else []),
        ])

    def require_static_rejections(self, context):
        case = self.case
        rules = case._work / "file.rules"
        disabled = case._work / "open-disabled.toml"
        try:
            for mode in ("fanotify", "open-disabled"):
                rules.write_text(f"static-read deny read {self.root / 'protected'}\n")
                log = case._work / f"static-{mode}.log"
                capabilities = tomllib.loads(case._patches()[-1].read_text())["capture"]["capabilities"]
                capabilities = [item for item in capabilities if item != "enforcement-file-permission-fanotify"]
                disabled.write_text(
                    f"[control]\nlog_path={json.dumps(str(log))}\n"
                    + (f"[capture]\ncapabilities={json.dumps(capabilities)}\n"
                       "[access_control.file.open]\nenabled=false\n" if mode == "open-disabled" else "")
                )
                patches = case._patches() + [disabled]
                runtime = ActrailRuntime(
                    case._config.repo, case._config.bin_dir, case._config.command_timeout_seconds,
                    context.output, case._work / "actraild.conf", operator_config_patches=patches,
                )
                case._runtime = runtime
                try:
                    runtime.prepare()
                except RuntimeError as error:
                    details = str(error) + (log.read_text() if log.exists() else "")
                    if "requires access_control.file.open backend lsm" not in details:
                        raise
                    context.report_progress(mode, "static read rule rejected at admission")
                else:
                    runtime.stop()
                    raise AssertionError(f"{mode} accepted an unsupported static read rule")
        finally:
            rules.write_text("")
            disabled.unlink()

    def run(self, context, results, port):
        case = self.case
        lsm = case._selection.name == "lsm"
        self.root.mkdir()
        resources = FileResources(self.root) if lsm else None
        if resources:
            resources.prepare()
        (self.root / "remove").mkdir()
        (self.root / "protected").write_bytes(b"." * 4096)
        (self.root / "truncate").write_bytes(b"t" * 4096)
        case._write_rules("allow", port)
        (case._work / "file.rules").write_text(
            f"static-read allow read {self.root / 'protected'}\n" if lsm else ""
        )
        case._write_patch()
        if not lsm:
            self.require_static_rejections(context)
        case._dynamic = DynamicPolicies(case._config, context.output, port, case._patches())
        case._runtime = case._dynamic.runtime
        case._dynamic.prepare()
        if not lsm:
            # Exercise actual host validation and publication through the loaded plugin.
            for operation in ("read", "write", "delete", "attr", "rename", "link"):
                case._dynamic._require_rejected({"rules": [{
                    "rule_id": f"semantics-{operation}", "operation": operation,
                    "decision": "deny", "path": str(self.root / "protected"), "priority": 20,
                }]}, "requires access_control.file.open backend lsm", domain="file")
        phases = {}
        if not lsm:
            phases["capabilities"] = TestResult(
                TestStatus.PASSED, "read/write/delete/attr/rename/link rejected without changing policy state",
            )
        results["semantics"] = TestResult(TestStatus.COMPOSITE, "file operation permissions", phases)
        with ThreadPoolExecutor(max_workers=1) as executor:
            agent = executor.submit(case._run_xiaoo, "semantics")
            try:
                for phase in self.phases:
                    self.wait(self.root / f"ready-{phase}", agent)
                    context.report_progress(phase, "real Xiaoo checks independent file permissions")
                    rules = [] if phase in ("empty", "restore") or (not lsm and phase != "open") else [{
                        "rule_id": f"semantics-{phase}", "operation": phase, "decision": "deny",
                        "path": f"{self.root}/remove/**" if phase == "delete" else str(self.root / "protected"), "priority": 20,
                    }]
                    document = {"rules": rules}
                    if lsm and phase == "write":
                        rules.append({"rule_id": "semantics-truncate", "operation": "write",
                                      "decision": "deny", "path": str(self.root / "truncate"), "priority": 20})
                    if resources and phase == "write":
                        rules.extend({"rule_id": f"resource-{label}", "operation": "open",
                                      "decision": "allow", "path": str(path), "priority": 20}
                                     for label, path in (("long", resources.long), ("short", resources.short)))
                    dynamic = case._dynamic
                    updated = dynamic.api.update_config(dynamic.owner("file"), document)
                    if updated.get("config") != document:
                        raise AssertionError(f"publication failed: {updated}")
                    before = (self.root / "protected").read_bytes()
                    (self.root / "truncate").write_bytes(b"t" * 4096)
                    for name in ("absolute", "relative"):
                        (self.root / "remove" / name).write_text("delete me")
                    (self.root / f"go-{phase}").touch()
                    destination = self.root / f"result-{phase}.json"
                    self.wait(destination, agent)
                    document = json.loads(destination.read_text())
                    self.verify(phase, document, before)
                    if resources:
                        resources.verify(document["resources"])
                    phases[phase] = TestResult(TestStatus.PASSED, "content access, mappings, deletion, attributes and truncation verified")
                trace = agent.result(timeout=case._config.launch_timeout_seconds)
                required = self.phases[1:-1] if lsm else ("open",)
                audits = {f"semantics-{op}": "deny" for op in required}
                if lsm:
                    audits["semantics-truncate"] = "deny"
                case._require_evidence(trace, "allow", audits)
                if resources:
                    output = case._runtime.run_checked(case._runtime.viewer_command(
                        "events", "--trace-id", str(trace), "--output-format", "json"), echo=False)
                    resources.verify_audit(json.loads(output.stdout)["events"])
                phases["audit"] = TestResult(TestStatus.PASSED, f"trace-{trace}: real agent, policy audits and captured payload")
            finally:
                for phase in self.phases:
                    (self.root / f"go-{phase}").touch()

    def wait(self, path, agent):
        deadline = time.monotonic() + self.case._config.launch_timeout_seconds
        while not path.exists():
            if agent.done():
                agent.result()
                raise AssertionError(f"Xiaoo finished without {path}")
            if time.monotonic() >= deadline:
                raise TimeoutError(f"Xiaoo did not produce {path}")
            time.sleep(self.case._poll)

    def verify(self, phase, document, before):
        effective_phase = phase if self.case._selection.name == "lsm" or phase == "open" else "restore"
        if str(self.case._xiaoo) not in {p["executable"] for p in document["processes"]}:
            raise AssertionError("workload did not descend from real Xiaoo")
        for name, observed in document["operations"].items():
            denied = (effective_phase == "open" and name.startswith("open-")) or (
                effective_phase == "write" and name in ("write", "open-write", "open-read-write",
                    "protected-mmap-write", "protected-mprotect-write")
            ) or (effective_phase == "write" and name.startswith("truncate-")) or (
                effective_phase == "read" and name not in ("write", "open-write")
                  and not name.startswith(("attr-", "delete-", "truncate-"))) or (
                effective_phase == "delete" and name.startswith("delete-")
            ) or (effective_phase == "attr" and name.startswith("attr-"))
            if observed["allowed"] == denied or (denied and observed.get("errno") not in (errno.EACCES, errno.EPERM)):
                raise AssertionError(f"{phase}: wrong {name} result: {observed}")
        expected = before if effective_phase == "write" else phase.encode() + before[len(phase):]
        if (self.root / "protected").read_bytes() != expected:
            raise AssertionError(f"{phase}: file contents differ from policy")
        expected_truncated = b"t" * 4096 if effective_phase == "write" else b""
        if (self.root / "truncate").read_bytes() != expected_truncated:
            raise AssertionError(f"{phase}: truncation effects differ from policy")
