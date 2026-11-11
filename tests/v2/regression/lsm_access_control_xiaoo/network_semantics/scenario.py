"""Publish independent protocol and operation policies during real agent traffic."""
from concurrent.futures import ThreadPoolExecutor
import errno
import json
from pathlib import Path
import shlex
import shutil
import sys

from tests.v2.common.core import TestResult, TestStatus
from ..continued.scenario import ContinuedAccessScenario
from ..dynamic import DynamicPolicies


class NetworkSemanticsScenario(ContinuedAccessScenario):
    phases = ("tcp", "udp", "connect", "bind", "listen", "restore")
    requires_lsm = True

    def __init__(self, case):
        super().__init__(case)
        self.root = case._work / "network-semantics"

    @staticmethod
    def command(case):
        return shlex.join([sys.executable, str(Path(__file__).with_name("workload.py")),
            "--root", str(case._work / "network-semantics"), "--xiaoo", str(case._xiaoo),
            "--timeout", str(case._config.launch_timeout_seconds), "--poll", str(case._poll)])

    def execute(self, context, results, port):
        case = self.case
        if self.root.exists():
            shutil.rmtree(self.root)
        self.root.mkdir()
        case._write_rules("allow", port)
        (case._work / "network.rules").write_text("")
        case._write_patch()
        case._dynamic = DynamicPolicies(case._config, context.output, port, case._patches())
        case._runtime = case._dynamic.runtime
        case._dynamic.prepare()
        dynamic = case._dynamic
        owner = dynamic.owner("network")
        dynamic.api.unload(owner)
        self.require_grant_boundaries()
        loaded = dynamic.api.load("network-policy-dynamic", owner, {
            "network_policy_rules_apply": [
                {"decision": decision, "operation": operation, "protocol": "any", "endpoint_scope": "*"}
                for decision in ("allow", "deny") for operation in ("connect", "send", "bind", "listen")
            ],
        })
        if loaded.get("plugin", {}).get("state") != "active":
            raise AssertionError(f"network plugin failed to load: {loaded}")
        self.require_policy_boundaries()
        (self.root / "endpoints.json").write_text(json.dumps(self.peers.start()))
        phases = {}
        results["network-semantics"] = TestResult(TestStatus.COMPOSITE, "network operation/protocol permissions", phases)
        lsm = case._selection.name == "lsm"
        with ThreadPoolExecutor(max_workers=1) as executor:
            agent = executor.submit(case._run_xiaoo, "network-semantics")
            try:
                for phase in self.phases:
                    self.wait(self.root / f"ready-{phase}", agent)
                    context.report_progress(phase, "real Xiaoo checks protocol and operation separation")
                    document = {"rules": self.rules(phase)}
                    if not lsm and self.requires_lsm and document["rules"]:
                        before = dynamic.api.config(owner)
                        for suffix in ("/validate", ""):
                            status, response = dynamic._config_response("network", suffix, document)
                            if not (status >= 400 or response.get("valid") is False) or "requires LSM" not in json.dumps(response):
                                raise AssertionError(f"non-LSM accepted new operation: {status} {response}")
                        if dynamic.api.config(owner) != before:
                            raise AssertionError("rejected rules changed configuration")
                    else:
                        dynamic._publish_network(owner, document["rules"])
                    (self.root / f"go-{phase}").touch()
                    destination = self.root / f"result-{phase}.json"
                    self.wait(destination, agent)
                    self.verify_network(phase if lsm or not self.requires_lsm else "restore", json.loads(destination.read_text()))
                    phases[phase] = TestResult(TestStatus.PASSED, "actual sends, peer data, connections, bind and listen effects verified")
                trace = agent.result(timeout=case._config.launch_timeout_seconds)
                required = {f"network-{phase}-{version}": "deny" for phase in self.phases[:-1] for version in (4, 6)} if lsm else {}
                case._require_evidence(trace, "allow", required)
                if lsm or not self.requires_lsm:
                    self.verify_audit(trace)
                self.verify_packets()
                phases["audit"] = TestResult(TestStatus.PASSED, f"trace-{trace}: actual traffic and rule audits")
            finally:
                for phase in self.phases:
                    (self.root / f"go-{phase}").touch()

    def rules(self, phase):
        if phase == "restore":
            return []
        operation = "send" if phase in ("tcp", "udp") else phase
        protocol = "udp" if phase == "udp" else "tcp"
        hosts = ((4, "127.0.0.3:*" if phase in ("bind", "listen") else "127.0.0.2:*"), (6, "[::1]:*"))
        if phase == "bind":
            hosts = tuple((version, endpoint.replace(":*", ":0")) for version, endpoint in hosts)
        if phase == "listen":
            local = json.loads((self.root / "locals.json").read_text())
            hosts = tuple((version, endpoint.replace(":*", f':{local[str(version)]}')) for version, endpoint in hosts)
        rules = [{"rule_id": f"network-{phase}-{version}", "decision": "deny", "operation": operation,
                  "protocol": protocol, "endpoint": endpoint} for version, endpoint in hosts]
        if operation in ("connect", "send"):
            for name, endpoint in self.peers.endpoints.items():
                if not name.startswith("control-"):
                    continue
                host = f'[{endpoint["host"]}]' if endpoint["version"] == 6 else endpoint["host"]
                rules.append({"rule_id": f"network-control-{name}", "decision": "allow",
                              "operation": operation, "endpoint": f'{host}:{endpoint["port"]}'})
        for rule in rules:
            if rule.get("operation") == "connect":
                del rule["operation"]
        return rules

    def require_policy_boundaries(self):
        pass

    def verify_audit(self, trace):
        runtime = self.case._runtime
        result = runtime.run_checked(runtime.viewer_command(
            "events", "--trace-id", str(trace), "--output-format", "json"), echo=False)
        observed = {}
        for event in json.loads(result.stdout)["events"]:
            payload = event.get("payload", {})
            if payload.get("decision") == "deny":
                observed.setdefault(payload.get("rule_id"), []).append(payload)
        for phase in self.phases[:-1]:
            operation = "send" if phase in ("tcp", "udp") else phase
            for rule in self.rules(phase)[:2]:
                side = "local_endpoint" if phase in ("bind", "listen") else "remote_endpoint"
                matches = observed.get(rule["rule_id"], [])
                expected = rule["endpoint"]
                if not any(payload.get("operation") == operation
                           and payload.get("metadata", {}).get("protocol") == rule["protocol"]
                           and (payload["metadata"].get(side, "").startswith(expected[:-1])
                                if expected.endswith("*") else payload["metadata"].get(side) == expected)
                           for payload in matches):
                    raise AssertionError(f"network audit lost operation/protocol/endpoint: {rule}: {matches}")

    def verify_network(self, phase, document):
        if str(self.case._xiaoo) not in {p["executable"] for p in document["processes"]}:
            raise AssertionError("workload did not descend from Xiaoo")
        for name, observed in document["operations"].items():
            denied = self.is_denied(phase, name)
            if observed["allowed"] == denied or (denied and observed.get("errno") not in (errno.EACCES, errno.EPERM)):
                raise AssertionError(f"{phase}: wrong {name}: {observed}")
            if observed["allowed"] and "peer" in observed:
                self.expected_packets.add((observed["peer"], observed["token"]))
        self.verify_packets()

    def is_denied(self, phase, name):
        return (phase in ("tcp", "udp") and name.startswith(f"protected-{phase}")) or (
            phase == "connect" and name.startswith("connect-protected-tcp")) or (
            phase == "bind" and name.startswith("bind-tcp")) or (
            phase == "listen" and name.startswith("listen-tcp"))

    def require_grant_boundaries(self):
        dynamic = self.case._dynamic
        owner = "wasm.network-policy-restricted"
        loaded = dynamic.api.load("network-policy-dynamic", owner, {"network_policy_rules_apply": [
            {"decision": "deny", "operation": "connect", "protocol": "tcp", "endpoint_scope": "127.0.0.2:*"}
        ]})
        if loaded.get("plugin", {}).get("state") != "active":
            raise AssertionError(f"restricted plugin failed to load: {loaded}")
        try:
            before = dynamic.api.config(owner)
            for operation, protocol, endpoint in (("send", "tcp", "127.0.0.2:1"),
                    ("bind", "tcp", "127.0.0.2:1"), ("connect", "any", "127.0.0.2:1"),
                    ("connect", "udp", "127.0.0.2:1"), ("connect", "tcp", "127.0.0.3:1")):
                candidate = {"rules": [{"rule_id": "grant-escape", "decision": "deny", "operation": operation,
                                       "protocol": protocol, "endpoint": endpoint}]}
                for suffix in ("/validate", ""):
                    status, response = dynamic._config_response("network", suffix, candidate, owner=owner)
                    if not (status >= 400 or response.get("valid") is False) or "missing network-policy.rules.apply grant" not in json.dumps(response):
                        raise AssertionError(f"grant accepted broader rule: {status} {response}")
                if dynamic.api.config(owner) != before:
                    raise AssertionError("grant rejection changed configuration")
        finally:
            dynamic.api.unload(owner)
