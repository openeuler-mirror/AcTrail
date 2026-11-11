"""Publish source, destination and overwritten-object permissions through plugins."""
from concurrent.futures import ThreadPoolExecutor
import errno
import json
import os
from pathlib import Path
import shlex
import sys

from tests.v2.common.core import TestResult, TestStatus
from ..dynamic import DynamicPolicies
from ..semantics.scenario import FileSemanticsScenario


class FileNamesScenario(FileSemanticsScenario):
    phases = ("initial", "source", "target", "overwrite", "restore")
    cases = ("rename", "replace", "exchange", "link", "relative-rename", "relative-link", "same-inode")

    def __init__(self, case):
        self.case = case
        self.root = case._work / "names"

    @staticmethod
    def command(case):
        return shlex.join([
            sys.executable, str(Path(__file__).with_name("workload.py")),
            "--root", str(case._work / "names"), "--xiaoo", str(case._xiaoo),
            "--timeout", str(case._config.launch_timeout_seconds), "--poll", str(case._poll),
        ])

    def run(self, context, results, port):
        case = self.case
        if case._selection.name != "lsm":
            raise ValueError("file name enforcement requires LSM")
        self.root.mkdir()
        for name in self.cases:
            (self.root / name).mkdir()
        case._write_rules("allow", port)
        case._write_patch()
        case._dynamic = DynamicPolicies(case._config, context.output, port, case._patches())
        case._runtime = case._dynamic.runtime
        case._dynamic.prepare()
        phases = {}
        results["names"] = TestResult(TestStatus.COMPOSITE, "source and destination file permissions", phases)
        with ThreadPoolExecutor(max_workers=1) as executor:
            agent = executor.submit(case._run_xiaoo, "names")
            try:
                for phase in self.phases:
                    self.wait(self.root / f"ready-{phase}", agent)
                    context.report_progress(phase, "real Xiaoo checks rename/link names and objects")
                    self.prepare(phase)
                    (self.root / f"go-{phase}").touch()
                    destination = self.root / f"result-{phase}.json"
                    self.wait(destination, agent)
                    document = json.loads(destination.read_text())
                    if str(case._xiaoo) not in {p["executable"] for p in document["processes"]}:
                        raise AssertionError("workload did not descend from real Xiaoo")
                    for name, observed in document["operations"].items():
                        denied = phase in ("source", "target") or (phase == "overwrite" and name == "replace")
                        if observed["allowed"] == denied or (denied and observed["errno"] not in (errno.EACCES, errno.EPERM)):
                            raise AssertionError(f"{phase}/{name}: {observed}")
                    phases[phase] = TestResult(TestStatus.PASSED, "syscall results, file names, contents and inodes verified")
                trace = agent.result(timeout=case._config.launch_timeout_seconds)
                required = {f"names-{phase}-{name}": "deny" for phase in ("source", "target") for name in self.cases}
                required["names-overwrite-replace"] = "deny"
                case._require_evidence(trace, "allow", required)
                phases["audit"] = TestResult(TestStatus.PASSED, f"trace-{trace}: source, target and overwritten object denials")
            finally:
                for phase in self.phases:
                    (self.root / f"go-{phase}").touch()

    def prepare(self, phase):
        rules = []
        for name in self.cases:
            directory = self.root / name
            for side in ("source", "target"):
                (directory / side).unlink(missing_ok=True)
            (directory / "source").write_text(f"{phase}/{name}/source")
            if name in ("replace", "exchange"):
                (directory / "target").write_text(f"{phase}/{name}/target")
            elif name == "same-inode":
                os.link(directory / "source", directory / "target")
            if phase in ("source", "target", "overwrite"):
                rules.append({
                    "rule_id": f"names-{phase}-{name}", "decision": "deny", "priority": 20,
                    "operation": "delete" if phase == "overwrite" else "link" if "link" in name else "rename",
                    "path": str(directory / ("target" if phase == "overwrite" else phase)),
                })
        dynamic = self.case._dynamic
        document = {"rules": rules}
        updated = dynamic.api.update_config(dynamic.owner("file"), document)
        if updated.get("config") != document:
            raise AssertionError(f"publication failed: {updated}")
