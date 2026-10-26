from __future__ import annotations

from http import HTTPStatus

import copy
import json
import os
import shutil
from pathlib import Path
from typing import Any
from urllib.error import HTTPError
from urllib.parse import urlencode
from urllib.request import ProxyHandler, Request, build_opener

from tests.v2.common.core import CommonTestConfig, TestOutput, TestResult, TestStatus
from tests.v2.common.plugin_test_environment import PluginRuntimeSpec, PluginTestEnvironment

from .constants import CASE_CONSTANTS


class DynamicPolicies(PluginTestEnvironment):
    """Publish real WIT host policies through the existing managed WASM packages."""

    def __init__(self, config: CommonTestConfig, output: TestOutput, port: int, patches: list[Path]):
        self._work = config.work_dir
        self._remote = f"127.0.0.2:{port}"
        self._extra_owners: list[str] = []
        super().__init__(
            config, output, operator_config=self._work / "actraild.conf",
            operator_config_patches=patches,
            web_host=CASE_CONSTANTS.loopback_host, web_port=CASE_CONSTANTS.init_web_port,
            plugin=PluginRuntimeSpec(
                package="command-policy-dynamic", instance_id=self.owner("command"),
                plugin_id=self.owner("command"), runtime="wasm",
                load_grants={"command_policy_rules_apply": [
                    {"decision": decision, "path_scope": f"{self._work}/**"}
                    for decision in ("allow", "deny", "gray")
                ]},
            ),
        )

    @staticmethod
    def owner(domain: str) -> str:
        return f"wasm.{domain}-policy-dynamic"

    def prepare(self) -> dict[str, Any]:
        target = Path(os.environ.get("CARGO_TARGET_DIR") or "target")
        if not target.is_absolute():
            target = self.config.repo / target
        for domain in ("command", "file", "network"):
            package = f"{domain}-policy-dynamic"
            source = self.config.repo / "examples/plugins/wit-component" / package
            destination = self._work / "plugins" / package
            destination.mkdir(parents=True, exist_ok=True)
            for original, installed in {
                "plugin.toml": f"{package}.plugin.toml",
                f"{package}.config.json": f"{package}.config.json",
                "config.schema.json": "config.schema.json",
            }.items():
                shutil.copy2(source / original, destination / installed)
            shutil.copy2(
                target / "wasm32-wasip2/release" / f"actrail_component_{domain}_policy_dynamic.wasm",
                destination / f"component-{package}.wasm",
            )
        self._prepare_network_peer()
        initial = super().prepare()
        for domain, grant, selector in (
            ("file", "path_scope", f"{self._work}/**"),
            ("network", "remote_scope", "127.0.0.2:*"),
        ):
            owner = self.owner(domain)
            response = self.api.load(
                f"{domain}-policy-dynamic", owner,
                {f"{domain}_policy_rules_apply": [
                    {"decision": decision, grant: selector} for decision in ("allow", "deny", "gray")
                ]},
            )
            self._extra_owners.append(owner)
            if response.get("plugin", {}).get("state") != "active":
                raise AssertionError(f"dynamic {domain} plugin failed to load: {response}")
        return initial

    def _prepare_network_peer(self) -> None:
        source = self._work / "plugins/network-policy-dynamic"
        destination = self._work / "plugins/network-policy-peer"
        destination.mkdir(parents=True, exist_ok=True)
        for name in ("component-network-policy-dynamic.wasm", "config.schema.json"):
            shutil.copy2(source / name, destination / name)
        manifest = (source / "network-policy-dynamic.plugin.toml").read_text(encoding="utf-8")
        manifest = manifest.replace('id = "wasm.network-policy-dynamic"', 'id = "wasm.network-policy-peer"', 1)
        (destination / "network-policy-peer.plugin.toml").write_text(manifest, encoding="utf-8")
        (destination / "network-policy-peer.config.json").write_text('{"rules": []}\n', encoding="utf-8")

    def publish(self, decision: str) -> dict[str, str]:
        documents = {
            "file": {"rules": [
                {"rule_id": "lsm-dynamic-open", "decision": decision, "operation": "open",
                 "path": f"{self._work}/protected/**", "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-dynamic-mkdir", "decision": decision, "operation": "mkdir",
                 "path": str(self._work / "create-target"), "priority": CASE_CONSTANTS.specific_rule_priority},
                {"rule_id": "lsm-dynamic-rmdir", "decision": decision, "operation": "rmdir",
                 "path": str(self._work / "remove-target"), "priority": CASE_CONSTANTS.specific_rule_priority},
            ]},
            "command": {"rules": [
                {"rule_id": "lsm-dynamic-command", "decision": decision,
                 "executable": str(self._work / "controlled-true"), "priority": CASE_CONSTANTS.specific_rule_priority},
            ]},
            "network": {"rules": [
                {"rule_id": "lsm-dynamic-network", "decision": decision, "remote": self._remote},
            ]},
        }
        required = {}
        for domain, document in documents.items():
            if decision == "gray":
                for rule in document["rules"]:
                    rule["gray_target"] = (
                        2147483647 if domain == "file" else "wasm.unavailable-gray-decider"
                    )
                    if domain == "network":
                        rule.update(timeout_ms=CASE_CONSTANTS.publish_timeout_ms, concurrency_limit=CASE_CONSTANTS.publish_concurrency_limit, fallback="allow")
            owner = self.owner(domain)
            validation = self.api.validate_config(owner, document)
            if validation.get("valid") is not True:
                raise AssertionError(f"dynamic {domain} validation failed: {validation}")
            updated = self.api.update_config(owner, document)
            if updated.get("config") != document:
                raise AssertionError(f"dynamic {domain} config differs from publication: {updated}")
            for rule in document["rules"]:
                required[rule["rule_id"]] = "deny" if decision == "gray" else decision
        self.require_decisions(decision)
        return required

    def publish_network_exception(self, decision: str) -> dict[str, str]:
        required = self.publish(decision)
        opposite = "deny" if decision == "allow" else "allow"
        self._publish_network(self.owner("network"), [
            {"rule_id": "lsm-dynamic-network-wildcard", "decision": opposite, "remote": "127.0.0.2:*"},
            {"rule_id": "lsm-dynamic-network", "decision": decision, "remote": self._remote},
        ])
        self.require_decisions(decision)
        fields = self._command_fields("network", ["rule", "dry-run", self._secondary_remote()])
        if fields.get("decision") != opposite or fields.get("rule_id") != "lsm-dynamic-network-wildcard":
            raise AssertionError(f"network wildcard did not cover the other port: {fields}")
        return required

    def require_network_owner_conflicts(self) -> None:
        owner = self.owner("network")
        peer = "wasm.network-policy-peer"
        original = self._current_config("network")
        loaded = self.api.load("network-policy-peer", peer, {
            "network_policy_rules_apply": [
                {"decision": decision, "remote_scope": "127.0.0.2:*"}
                for decision in ("allow", "deny")
            ],
        })
        self._extra_owners.append(peer)
        if loaded.get("plugin", {}).get("state") != "active":
            raise AssertionError(f"second network owner failed to load: {loaded}")
        endpoint = {"rule_id": "lsm-dynamic-network", "decision": "deny", "remote": self._remote}
        wildcard = {"rule_id": "lsm-dynamic-network-wildcard", "decision": "allow", "remote": "127.0.0.2:*"}
        peer_endpoint = {"rule_id": "peer-network", "decision": "deny", "remote": self._secondary_remote()}
        self._publish_network(owner, [endpoint])
        self._publish_network(peer, [peer_endpoint])
        self._require_network_rejected(owner, [endpoint, wildcard], peer)
        self._publish_network(peer, [])
        self._publish_network(owner, [wildcard, endpoint])
        self._require_network_rejected(peer, [peer_endpoint], peer)
        self._require_network_rejected(peer, [{**peer_endpoint, "remote": self._remote}], peer)
        unloaded = self.api.unload(peer)
        if unloaded.get("plugin", {}).get("state") == "active":
            raise AssertionError(f"second network owner remained active: {unloaded}")
        self._extra_owners.remove(peer)
        self._publish_network(owner, original["rules"])

    def _secondary_remote(self) -> str:
        return "127.0.0.2:1" if self._remote != "127.0.0.2:1" else "127.0.0.2:2"

    def _publish_network(self, owner: str, rules: list[dict]) -> None:
        candidate = {"rules": rules}
        validation = self.api.validate_config(owner, candidate)
        if validation.get("valid") is not True:
            raise AssertionError(f"network candidate validation failed: {validation}")
        updated = self.api.update_config(owner, candidate)
        if updated.get("config") != candidate:
            raise AssertionError(f"network publication changed its candidate: {updated}")

    def _network_snapshot(self, peer: str) -> dict:
        return {
            "owners": {owner: {
                "config": self.api.config(owner).get("config"),
                "list": self._command_output(owner, ["rule", "list"]),
            } for owner in (self.owner("network"), peer)},
            "matches": [self._command_fields("network", ["rule", "dry-run", remote])
                        for remote in (self._remote, self._secondary_remote())],
        }

    def _require_network_rejected(self, owner: str, rules: list[dict], peer: str) -> None:
        before = self._network_snapshot(peer)
        for suffix in ("/validate", ""):
            status, response = self._config_response("network", suffix, {"rules": rules}, owner=owner)
            rejected = status >= HTTPStatus.BAD_REQUEST or (suffix == "/validate" and response.get("valid") is False)
            if not rejected or not any(fragment in json.dumps(response) for fragment in ("overlap", "owned")):
                raise AssertionError(f"network owner conflict was not rejected: {status} {response}")
            if self._network_snapshot(peer) != before:
                raise AssertionError("rejected network publication changed config, rules, or revisions")

    def require_decisions(self, decision: str) -> None:
        for domain, argv in (
            ("file", ["rule", "dry-run", str(self._work / "protected/file.marker"), "--operation", "open"]),
            ("file", ["rule", "dry-run", str(self._work / "create-target"), "--operation", "mkdir"]),
            ("file", ["rule", "dry-run", str(self._work / "remove-target"), "--operation", "rmdir"]),
            ("command", ["rule", "dry-run", str(self._work / "controlled-true")]),
            ("network", ["rule", "dry-run", self._remote]),
        ):
            fields = self._command_fields(domain, argv)
            if fields.get("matched") != "true" or fields.get("decision") != decision:
                raise AssertionError(f"dynamic {domain} dry-run is wrong: {fields}")

    def require_argv_rejected(self) -> None:
        candidate = copy.deepcopy(self.current_config())
        candidate["rules"][0]["args"] = ["--restricted"]
        self._require_rejected(candidate, "executable-only")

    def require_identifier_rejected(self) -> None:
        candidate = copy.deepcopy(self.current_config())
        candidate["rules"][0]["rule_id"] += "\0hidden"
        self._require_rejected(candidate, "NUL")

    def require_capacity_rejected(self, capacity: int) -> None:
        candidate = self._current_config("file")
        # An any-operation recursive scope compiles into six distinct map entries.
        # Keep the WASM fixture work bounded while exceeding the real kernel capacity.
        for index in range(capacity // 6 + 1):
            candidate["rules"].append({
                "rule_id": f"lsm-overflow-{index}", "decision": "deny",
                "operation": "any", "path": f"{self._work}/overflow-{index}/**", "priority": CASE_CONSTANTS.priority,
            })
        self._require_rejected(candidate, "policy_max_entries", domain="file")

    def _current_config(self, domain: str) -> dict[str, Any]:
        document = self.api.config(self.owner(domain))
        if not isinstance(document.get("config"), dict):
            raise AssertionError(f"{domain} config response omitted its document: {document}")
        return copy.deepcopy(document["config"])

    def _require_rejected(self, candidate: dict[str, Any], reason: str, *, domain: str = CASE_CONSTANTS.require_rejected_domain) -> None:
        before_config = self._current_config(domain)
        query = ["rule", "dry-run", str(self._work / "controlled-true")] if domain == "command" else [
            "rule", "dry-run", str(self._work / "protected/file.marker"), "--operation", "open"
        ]
        before = self._command_fields(domain, query)
        for suffix in ("/validate", ""):
            status, response = self._config_response(domain, suffix, candidate)
            rejected = status >= HTTPStatus.BAD_REQUEST or (suffix == "/validate" and response.get("valid") is False)
            if not rejected or reason not in json.dumps(response):
                raise AssertionError(f"LSM {suffix or 'submit'} did not reject {reason}: {status} {response}")
            print(f"LSM candidate rejection domain={domain} operation={suffix or '/submit'} status={status} body={response}")
        after = self._command_fields(domain, query)
        if self._current_config(domain) != before_config or after != before:
            raise AssertionError(f"rejected {reason} candidate changed policy state: {before} -> {after}")
        self.require_decisions("deny")

    def _config_response(
        self, domain: str, suffix: str, candidate: dict[str, Any], *, owner: str | None = None,
    ) -> tuple[int, dict]:
        # Keep the real error body: PluginWebApi's normal curl --fail path drops it.
        url = self.api._base_url + "/api/plugins/runtime/config" + suffix
        url += "?" + urlencode({"instance_id": owner or self.owner(domain)})
        request = Request(url, data=json.dumps({"config": candidate}).encode(), method="POST",
                          headers={"Content-Type": "application/json"})
        opener = build_opener(ProxyHandler({}))
        try:
            with opener.open(request, timeout=self.config.command_timeout_seconds) as response:
                return response.status, self._response_document(response)
        except HTTPError as error:
            with error:
                return error.code, self._response_document(error)

    @staticmethod
    def _response_document(response: Any) -> dict:
        raw = response.read().decode("utf-8")
        if response.headers.get_content_type() == "application/json":
            return json.loads(raw)
        return {"error": raw}

    def _command_fields(self, domain: str, argv: list[str]) -> dict[str, str]:
        output = self._command_output(self.owner(domain), argv)
        return dict(item.split("=", 1) for item in output.split() if "=" in item)

    def _command_output(self, owner: str, argv: list[str]) -> str:
        response = self.api.command(owner, argv)
        command = response.get("command", {})
        if command.get("exit_code") != 0:
            raise AssertionError(f"policy dry-run failed: {response}")
        return command.get("stdout", "")

    def unload_all(self) -> None:
        for owner in list(self._extra_owners):
            response = self.api.unload(owner)
            if response.get("plugin", {}).get("state") == "active":
                raise AssertionError(f"policy owner remained active: {response}")
            self._extra_owners.remove(owner)
        response = self.unload_plugin()
        if response.get("plugin", {}).get("state") == "active":
            raise AssertionError(f"command policy owner remained active: {response}")

    def cleanup(self) -> TestResult:
        failures = []
        for owner in list(self._extra_owners):
            try:
                self.api.unload(owner)
                self._extra_owners.remove(owner)
            except Exception as error:
                failures.append(str(error))
        base = super().cleanup()
        if not failures:
            return base
        return TestResult(TestStatus.COMPOSITE, "LSM dynamic policies cleanup", {
            "runtime": base, "owners": TestResult(TestStatus.FAILED, "; ".join(failures)),
        })
