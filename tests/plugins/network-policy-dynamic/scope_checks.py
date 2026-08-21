"""Grant and selector boundary checks for the dynamic network-policy E2E."""

from __future__ import annotations

from tests.v2.common.plugin_web_api import PluginWebApi


PACKAGE = "network-policy-dynamic"
EXACT_GRANT_INSTANCE = "wasm.network-policy-exact-grant"


class NetworkPolicyScopeVerifier:
    def __init__(
        self,
        api: PluginWebApi,
        primary_instance: str,
        endpoint: str,
        remote_scope: str,
    ) -> None:
        self.api = api
        self.primary_instance = primary_instance
        self.endpoint = endpoint
        self.remote_scope = remote_scope

    def require_grant_containment(self) -> None:
        exact_candidate = {
            "rules": [
                {
                    "rule_id": "exact-covered-by-any-port-grant",
                    "decision": "deny",
                    "remote": self.endpoint,
                }
            ]
        }
        validation = self.api.validate_config(self.primary_instance, exact_candidate)
        if validation.get("valid") is not True:
            raise AssertionError(
                f"IP:* grant did not cover a same-IP exact endpoint: {validation}"
            )

        self._require_unloaded(self.primary_instance, "any-port publisher")
        loaded = self.api.load(
            PACKAGE,
            EXACT_GRANT_INSTANCE,
            {
                "network_policy_rules_apply": [
                    {"decision": "deny", "remote_scope": self.endpoint}
                ]
            },
        )
        plugin = loaded.get("plugin")
        if not isinstance(plugin, dict) or plugin.get("state") != "active":
            raise AssertionError(f"exact-grant publisher load failed: {loaded}")
        wildcard_candidate = {
            "rules": [
                {
                    "rule_id": "wildcard-not-covered-by-exact-grant",
                    "decision": "deny",
                    "remote": self.remote_scope,
                }
            ]
        }
        validation = self.api.validate_config(EXACT_GRANT_INSTANCE, wildcard_candidate)
        errors = validation.get("errors")
        if validation.get("valid") is not False or not isinstance(errors, list):
            raise AssertionError(f"exact grant covered an IP:* rule: {validation}")
        if not any(
            "missing network-policy.rules.apply grant" in str(error)
            for error in errors
        ):
            raise AssertionError(f"exact grant rejection reason is missing: {validation}")
        self._require_unloaded(EXACT_GRANT_INSTANCE, "exact-grant publisher")

    def require_selector_validation(self) -> None:
        ipv6_candidate = {
            "rules": [
                {
                    "rule_id": "ipv6-any-port",
                    "decision": "deny",
                    "remote": "[::1]:*",
                }
            ]
        }
        validation = self.api.validate_config(self.primary_instance, ipv6_candidate)
        errors = validation.get("errors")
        if validation.get("valid") is not False or not isinstance(errors, list):
            raise AssertionError(f"out-of-grant IPv6 selector was accepted: {validation}")
        if not any(
            "missing network-policy.rules.apply grant" in str(error)
            for error in errors
        ):
            raise AssertionError(
                f"valid IPv6 selector was not parsed before grant check: {validation}"
            )

        for remote, fragment in (("::1:*", "must bracket"), ("*", "shorter than 3")):
            candidate = {
                "rules": [
                    {
                        "rule_id": "invalid-selector",
                        "decision": "deny",
                        "remote": remote,
                    }
                ]
            }
            validation = self.api.validate_config(self.primary_instance, candidate)
            errors = validation.get("errors")
            if validation.get("valid") is not False or not isinstance(errors, list):
                raise AssertionError(
                    f"invalid rule selector {remote} was accepted: {validation}"
                )
            if not any(fragment in str(error) for error in errors):
                raise AssertionError(
                    f"invalid selector {remote} missed error fragment {fragment}: {validation}"
                )

    def _require_unloaded(self, instance: str, label: str) -> None:
        unloaded = self.api.unload(instance)
        plugin = unloaded.get("plugin")
        if not isinstance(plugin, dict) or plugin.get("state") == "active":
            raise AssertionError(f"{label} remained active: {unloaded}")
