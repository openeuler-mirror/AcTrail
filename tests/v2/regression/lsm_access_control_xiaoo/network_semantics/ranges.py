"""Exercise CIDR/port scopes through real network effects and plugin publication."""
import ipaddress
import json
from .scenario import NetworkSemanticsScenario
from .range_peers import RangePeers


class NetworkRangesScenario(NetworkSemanticsScenario):
    requires_lsm = False

    def __init__(self, case):
        super().__init__(case)
        self.peers.stop()
        self.peers = RangePeers(case._timeout, case._poll)

    def rules(self, phase):
        if phase == "restore" or (self.case._selection.name != "lsm" and phase != "connect"):
            return []
        rules = [rule for rule in super().rules(phase) if rule["decision"] == "deny"]
        for rule in rules:
            if rule["decision"] == "allow":
                continue
            host, port = rule["endpoint"].rsplit(":", 1)
            cidr = "[::/126]" if host.startswith("[") else "127.0.0.2/31"
            if phase == "bind":
                cidr = "[::/0]" if host.startswith("[") else "0.0.0.0/0"
            ports = "1-65535" if port == "*" else f"{max(0, int(port)-1)}-{min(65535, int(port)+1)}"
            if phase in ("tcp", "udp", "connect"):
                version = 6 if host.startswith("[") else 4
                first = self.peers.endpoints[f"protected-tcp{version}-low"]["port"]
                last = self.peers.endpoints[f"protected-tcp{version}-high"]["port"]
                ports = f"{first}-{last}"
            rule["endpoint"] = f"{cidr}:{ports}"
            if phase == "connect":
                rule.pop("protocol")
        if phase in ("tcp", "udp"):
            for version in (4, 6):
                host = "[::1]" if version == 6 else "127.0.0.2"
                cidr = "[::/126]" if version == 6 else "127.0.0.2/31"
                middle = self.peers.endpoints[f"protected-tcp{version}-middle"]["port"]
                high = self.peers.endpoints[f"protected-tcp{version}-high"]["port"]
                # A more specific IP wins even with a wider port range. At the
                # same IP prefix, a narrower interval wins over the deny range.
                endpoint = f"{host}:{high}-{min(65535, high+10)}" if phase == "tcp" else f"{cidr}:{middle}"
                rules.append({"rule_id": f"range-exception-{version}", "decision": "allow",
                              "operation": "send", "protocol": phase, "endpoint": endpoint})
        return rules

    def is_denied(self, phase, name):
        if phase == "connect":
            return name.startswith("connect-protected-")
        if self.case._selection.name != "lsm":
            return False
        if (phase == "tcp" and name.startswith("protected-tcp") and "-high-" in name) or (
                phase == "udp" and name.startswith("protected-udp") and "-middle-" in name):
            return False
        return super().is_denied(phase, name)

    def require_policy_boundaries(self):
        from .range_publication import RangePublication
        RangePublication(self.case).verify()

    def verify_audit(self, trace):
        if self.case._selection.name != "lsm":
            self.case._require_evidence(trace, "allow", {f"network-connect-{version}": "deny" for version in (4, 6)})
            return
        runtime = self.case._runtime
        result = runtime.run_checked(runtime.viewer_command(
            "events", "--trace-id", str(trace), "--output-format", "json"), echo=False)
        for event in json.loads(result.stdout)["events"]:
            payload = event.get("payload", {})
            rule_id = payload.get("rule_id", "") or ""
            if not rule_id.startswith("network-") or payload.get("decision") != "deny":
                continue
            phase = rule_id.split("-")[1]
            metadata = payload.get("metadata", {})
            key = "local_endpoint" if phase in ("bind", "listen") else "remote_endpoint"
            endpoint = metadata.get(key, "")
            host, port = endpoint.rsplit(":", 1)
            ipaddress.ip_address(host.strip("[]"))
            if not 0 <= int(port) <= 65535 or metadata.get("protocol") not in ("tcp", "udp"):
                raise AssertionError(f"range audit lost actual endpoint: {payload}")

    def require_grant_boundaries(self):
        dynamic = self.case._dynamic
        owner = "wasm.network-policy-restricted"
        loaded = dynamic.api.load(dynamic.api.package_key("wasm.network-policy-dynamic"), owner, {"network_policy_rules_apply": [
            {"decision": decision, "endpoint_scope": endpoint}
            for decision in ("allow", "deny")
            for endpoint in ("127.0.0.2/31:100-200", "[::/126]:100-200")
        ]})
        if loaded.get("plugin", {}).get("state") != "active":
            raise AssertionError(loaded)
        try:
            for endpoint in ("127.0.0.2:100", "127.0.0.3:200", "[::1]:100-200", "[::2/127]:120-180"):
                dynamic._publish_network(owner, [{"rule_id": "range-granted", "decision": "deny", "endpoint": endpoint}])
            before = dynamic.api.config(owner)
            for endpoint in ("127.0.0.0/30:100-200", "127.0.0.2:99-200", "127.0.0.2:100-201",
                             "127.0.0.4:100", "[::/125]:100-200", "[::4]:100", "[::1]:*"):
                candidate = {"rules": [{"rule_id": "range-escape", "decision": "deny", "endpoint": endpoint}]}
                for suffix in ("/validate", ""):
                    status, response = dynamic._config_response("network", suffix, candidate, owner=owner)
                    if not (status >= 400 or response.get("valid") is False) or "missing network-policy.rules.apply grant" not in json.dumps(response):
                        raise AssertionError(f"range grant escaped: {endpoint}: {status} {response}")
                if dynamic.api.config(owner) != before:
                    raise AssertionError("range grant rejection changed configuration")
        finally:
            dynamic.api.unload(owner)
