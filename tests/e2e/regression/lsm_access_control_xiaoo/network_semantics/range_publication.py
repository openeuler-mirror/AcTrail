"""Validate live range publication, precedence, ownership and snapshot capacity."""
import json


class RangePublication:
    def __init__(self, case):
        self.case = case
        self.dynamic = case._dynamic
        self.owner = self.dynamic.owner("network")

    def verify(self):
        rules = []
        for version, scopes in ((4, ("0.0.0.0/0", "127.0.0.0/8", "127.0.0.2")),
                                (6, ("[::/0]", "[2001:db8::/32]", "[2001:db8::2]"))):
            for label, decision, endpoint in (("all", "deny", f"{scopes[0]}:*"),
                    ("network", "allow", f"{scopes[1]}:100-200"),
                    ("ports", "deny", f"{scopes[1]}:150-160"),
                    ("host", "allow", f"{scopes[2]}:100-200")):
                rules.append({"rule_id": f"range-{version}-{label}", "decision": decision, "endpoint": endpoint})
        self.dynamic._publish_network(self.owner, rules)
        for endpoint, rule in (("0.0.0.0:0", "4-all"), ("192.0.2.1:65535", "4-all"),
                ("127.1.1.1:155", "4-ports"), ("127.1.1.1:170", "4-network"),
                ("127.0.0.2:155", "4-host"), ("[::]:0", "6-all"),
                ("[ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff]:65535", "6-all"),
                ("[2001:db8::1]:155", "6-ports"), ("[2001:db8::1]:170", "6-network"),
                ("[2001:db8::2]:155", "6-host")):
            self.expect(endpoint, "range-" + rule)
        if self.case._selection.name == "lsm":
            rules.append({"rule_id": "range-tcp", "decision": "deny", "protocol": "tcp", "endpoint": "127.0.0.0/8:100-200"})
            self.dynamic._publish_network(self.owner, rules)
            self.expect("127.1.1.1:170", "range-tcp", "tcp")
            self.expect("127.1.1.1:170", "range-4-network", "udp")
            # Each interval expands into 30 kernel keys. The configured capacity
            # must reject the complete candidate and preserve the active rules.
            oversized = [{"rule_id": f"overflow-{i}", "decision": "deny", "endpoint": f"192.0.2.{i}:1-65534"}
                         for i in range(self.case._capacity // 30 + 1)]
            self.reject(oversized, "policy_max_entries")
            self.expect("127.1.1.1:170", "range-tcp", "tcp")
        self.dynamic._publish_network(self.owner, [])
        peer = "wasm.network-policy-peer"
        self.dynamic.api.load(self.dynamic.api.package_key(peer), peer, {"network_policy_rules_apply": [
            {"decision": "deny", "endpoint_scope": "*"}]})
        try:
            self.dynamic._publish_network(self.owner, [{"rule_id": "range-owner", "decision": "deny", "endpoint": "127.0.0.0/8:100-200"}])
            self.reject([{"rule_id": "range-peer", "decision": "deny", "endpoint": "127.0.0.2/31:199-250"}], "overlap", owner=peer)
            self.dynamic._publish_network(peer, [{"rule_id": "range-peer", "decision": "deny", "endpoint": "127.0.0.2/31:201-250"}])
        finally:
            self.dynamic.api.unload(peer)
            self.dynamic._publish_network(self.owner, [])

    def expect(self, endpoint, rule, protocol="any"):
        fields = self.dynamic._command_fields("network", ["rule", "dry-run", endpoint, "--protocol", protocol])
        if fields.get("rule_id") != rule:
            raise AssertionError(f"range precedence: {endpoint}: expected {rule}, got {fields}")

    def reject(self, rules, reason, owner=None):
        owner = owner or self.owner
        before = self.dynamic.api.config(owner)
        for suffix in ("/validate", ""):
            status, response = self.dynamic._config_response("network", suffix, {"rules": rules}, owner=owner)
            if not (status >= 400 or response.get("valid") is False) or reason not in json.dumps(response):
                raise AssertionError(f"range publication accepted {reason}: {status} {response}")
            if self.dynamic.api.config(owner) != before:
                raise AssertionError("rejected range publication changed configuration")
