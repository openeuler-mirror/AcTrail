"""Publish real policies while Xiaoo retains open file descriptors."""

from concurrent.futures import ThreadPoolExecutor
import errno
import json
import os
import shlex
import stat
import sys
import time
from pathlib import Path

from tests.v2.common.core import TestResult, TestStatus
from ..dynamic import DynamicPolicies
from .peers import NetworkPeers


class ContinuedAccessScenario:
    def __init__(self, case):
        self.case = case
        self.root = case._work / "continued"
        self.fastopen = os.environ.get("LSM_ACCESS_CONTROL_XIAOO_E2E_FASTOPEN") == "1"
        self.peers = NetworkPeers(case._timeout, case._poll, fastopen=self.fastopen)
        self.expected_packets = set()

    @staticmethod
    def command(case):
        return shlex.join([
            sys.executable, str(Path(__file__).with_name("workload.py")),
            "--root", str(case._work / "continued"), "--xiaoo", str(case._xiaoo),
            "--timeout", str(case._config.launch_timeout_seconds), "--poll", str(case._poll),
        ])

    def run(self, context, results, port):
        try:
            self.execute(context, results, port)
        finally:
            self.peers.stop()

    def execute(self, context, results, port):
        case = self.case
        if case._selection.name != "lsm":
            raise ValueError("continued access requires the LSM backend")
        self.root.mkdir()
        for name in ("protected", "control"):
            (self.root / name).write_bytes(b"." * 4096)
            (self.root / f"remove-{name}").mkdir()
        case._write_rules("allow", port)
        (case._work / "network.rules").write_text("")
        case._write_patch()
        case._dynamic = DynamicPolicies(case._config, context.output, port, case._patches())
        case._runtime = case._dynamic.runtime
        case._dynamic.prepare()
        dynamic = case._dynamic
        owner = dynamic.owner("network")
        dynamic.api.unload(owner)
        loaded = dynamic.api.load("network-policy-dynamic", owner, {
            "network_policy_rules_apply": [
                {"decision": decision, "operation": "send", "endpoint_scope": remote}
                for decision in ("allow", "deny") for remote in ("127.0.0.2:*", "[::1]:*")
            ],
        })
        if loaded.get("plugin", {}).get("state") != "active":
            raise AssertionError(f"network plugin failed to load: {loaded}")
        (self.root / "endpoints.json").write_text(json.dumps(self.peers.start()))
        phases = {}
        results["continued"] = TestResult(TestStatus.COMPOSITE, "retained files, mappings and sockets", phases)
        with ThreadPoolExecutor(max_workers=1) as executor:
            agent = executor.submit(case._run_xiaoo, "continued")
            try:
                for phase in ("initial", "deny", "restore"):
                    self.wait(self.root / f"ready-{phase}", agent)
                    context.report_progress(phase, "real Xiaoo uses the same FDs across policy publication")
                    if phase != "initial":
                        self.publish("deny" if phase == "deny" else "allow")
                    for role in ("protected", "control"):
                        for mode in ("absolute", "relative"):
                            (self.root / f"remove-{role}" / mode).write_text("remove me")
                    before = (self.root / "protected").read_bytes()
                    (self.root / f"go-{phase}").touch()
                    path = self.root / f"result-{phase}.json"
                    self.wait(path, agent)
                    document = json.loads(path.read_text())
                    self.verify(phase, document, before)
                    phases[phase] = TestResult(TestStatus.PASSED, "file access, unlink, chmod/chown and TCP/UDP effects verified")
                    if self.fastopen:
                        phases[f"fastopen_{phase}"] = TestResult(
                            TestStatus.PASSED, "IPv4/IPv6 MSG_FASTOPEN decisions, SYN-data acknowledgement and peer payloads verified",
                        )
                trace = agent.result(timeout=case._config.launch_timeout_seconds)
                required = {"continued-file": "deny", "continued-unlink": "deny", "continued-network4": "deny",
                            "continued-network6": "deny", "continued-protected-udp4": "deny",
                            "continued-protected-tcp6": "deny"}
                case._require_evidence(trace, "allow", required)
                self.verify_packets()
                phases["audit"] = TestResult(TestStatus.PASSED, f"trace-{trace}: real agent ancestry, LSM denial and payload verified")
            finally:
                for phase in ("initial", "deny", "restore"):
                    (self.root / f"go-{phase}").touch()

    def publish(self, decision):
        dynamic = self.case._dynamic
        document = {"rules": [{"rule_id": "continued-file", "decision": decision,
                               "operation": "any", "path": str(self.root / "protected"),
                               "priority": 20},
                              {"rule_id": "continued-unlink", "decision": decision,
                               "operation": "any", "path": f"{self.root}/remove-protected/**",
                               "priority": 20}]}
        updated = dynamic.api.update_config(dynamic.owner("file"), document)
        if updated.get("config") != document:
            raise AssertionError(f"file publication failed: {updated}")
        rules = [{"rule_id": f"continued-network{version}", "decision": decision, "endpoint": remote}
                 for version, remote in ((4, "127.0.0.2:*"), (6, "[::1]:*"))]
        for name, endpoint in self.peers.endpoints.items():
            if name.startswith("control-") or name in ("protected-udp4", "protected-tcp6"):
                host = f'[{endpoint["host"]}]' if endpoint["version"] == 6 else endpoint["host"]
                rules.append({"rule_id": f"continued-{name}",
                              "decision": "allow" if name.startswith("control-") else decision,
                              "endpoint": f'{host}:{endpoint["port"]}'})
        for rule in rules:
            rule["operation"] = "send"
        status, response = dynamic._config_response("network", "/validate", {"rules": rules})
        if status >= 400 or response.get("valid") is False:
            raise AssertionError(f"continued network validation failed: {status} {response}")
        dynamic._publish_network(dynamic.owner("network"), rules)

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
        if str(self.case._xiaoo) not in {process["executable"] for process in document["processes"]}:
            raise AssertionError("workload did not descend from real Xiaoo")
        for name, observed in document["operations"].items():
            allowed = phase != "deny" or name.startswith("control-")
            if observed["allowed"] != allowed or (
                not allowed and observed.get("errno") not in (errno.EACCES, errno.EPERM)
            ):
                raise AssertionError(f"{phase}: wrong {name} result: {observed}")
            if allowed and "peer" in observed:
                self.expected_packets.add((observed["peer"], observed["token"]))
        actual = (self.root / "protected").read_bytes()
        expected = before if phase == "deny" else phase.encode() + before[len(phase):]
        if actual != expected:
            raise AssertionError(f"{phase}: protected file effects differ from policy")
        if not (self.root / "control").read_bytes().startswith(phase.encode()):
            raise AssertionError(f"{phase}: unrelated file access was affected")
        for role in ("protected", "control"):
            expected_mode = 0o640 if phase == "deny" and role == "protected" else {
                "initial": 0o640, "deny": 0o604, "restore": 0o600,
            }[phase]
            if stat.S_IMODE((self.root / role).stat().st_mode) != expected_mode:
                raise AssertionError(f"{phase}: unexpected file mode for {role}")
            expected_owner = 65534 if phase == "deny" and role == "protected" else {
                "initial": 65534, "deny": 65533, "restore": os.getuid(),
            }[phase]
            actual = (self.root / role).stat()
            if (actual.st_uid, actual.st_gid) != (expected_owner, expected_owner):
                raise AssertionError(f"{phase}: unexpected ownership for {role}")
            for mode in ("absolute", "relative"):
                exists = (self.root / f"remove-{role}" / mode).exists()
                if exists != (phase == "deny" and role == "protected"):
                    raise AssertionError(f"{phase}: unexpected unlink effect for {role}/{mode}")
        self.verify_packets()

    def verify_packets(self):
        deadline = time.monotonic() + self.case._timeout
        while not self.expected_packets.issubset(self.peers.received):
            if self.peers.failure or time.monotonic() >= deadline:
                raise AssertionError(f"missing network effects: {self.expected_packets - self.peers.received}; {self.peers.failure}")
            time.sleep(self.case._poll)
        if self.peers.received != self.expected_packets:
            raise AssertionError(f"denied data reached peer: {self.peers.received - self.expected_packets}")
