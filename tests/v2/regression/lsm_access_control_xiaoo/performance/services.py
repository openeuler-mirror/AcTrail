from __future__ import annotations

import argparse
import os
import shutil
import socket
import threading
import time
from pathlib import Path

from tests.v2.common.core import CommonTestConfig, TestOutput
from tests.v2.common.plugin_test_environment import PluginRuntimeSpec, PluginTestEnvironment
from .paths import REPO

from .constants import CASE_CONSTANTS


class Peer:
    def __init__(self, timeout: float, backlog: int):
        self._socket = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        self._socket.bind((CASE_CONSTANTS.loopback_host, 0))
        self._socket.listen(backlog)
        self._socket.settimeout(timeout)
        self.port = self._socket.getsockname()[1]
        self.accepted = 0
        self.failure: str | None = None
        self._stop = threading.Event()
        self._thread = threading.Thread(target=self._serve, daemon=True)

    def start(self) -> None:
        self._thread.start()

    def _serve(self) -> None:
        try:
            while not self._stop.is_set():
                try:
                    connection, _ = self._socket.accept()
                except socket.timeout:
                    continue
                connection.close()
                self.accepted += 1
        except OSError as error:
            if not self._stop.is_set():
                self.failure = str(error)

    def stop(self) -> None:
        self._stop.set()
        self._socket.close()
        self._thread.join(timeout=CASE_CONSTANTS.stop_timeout)
        if self._thread.is_alive() or self.failure:
            raise RuntimeError(f"TCP peer cleanup failed: {self.failure}")


class WasmPublisher(PluginTestEnvironment):
    """Publish measured endpoint rules through the installed official WASM package."""

    def __init__(self, args: argparse.Namespace, work: Path, output: TestOutput, ports: list[int], patches: list[Path]):
        config = CommonTestConfig(
            repo=REPO, bin_dir=args.bin_dir, work_dir=work,
            command_timeout_seconds=args.command_timeout,
            launch_timeout_seconds=args.launch_timeout,
            drain_attempts=max(1, int(args.timeout / args.peer_poll_seconds)),
            drain_interval_seconds=args.peer_poll_seconds,
        )
        self._candidate = {"rules": [
            {"rule_id": f"bench-{decision}", "decision": decision, "remote": f"127.0.0.1:{port}"}
            for decision, port in zip(("allow", "deny"), ports)
        ]}
        super().__init__(
            config, output, operator_config=work / "actraild.conf",
            operator_config_patches=patches, web_host=CASE_CONSTANTS.loopback_host, web_port=CASE_CONSTANTS.init_web_port,
            plugin=PluginRuntimeSpec(
                package="network-policy-dynamic", instance_id="wasm.network-policy-dynamic",
                plugin_id="wasm.network-policy-dynamic", runtime="wasm",
                load_grants={"network_policy_rules_apply": [
                    {"decision": rule["decision"], "remote_scope": rule["remote"]}
                    for rule in self._candidate["rules"]
                ]},
            ),
        )
        self._package = Path(os.environ.get("ACTRAIL_PLUGIN_DIR", CASE_CONSTANTS.actrail_plugin_dir)).expanduser() / self.plugin.package

    def _dry_run(self, remote: str) -> dict[str, str]:
        response = self.api.command(self.plugin.instance_id, ["rule", "dry-run", remote])
        command = response.get("command", {})
        if command.get("exit_code") != 0:
            raise AssertionError(f"real WASM dry-run failed: {response}")
        return dict(item.split("=", 1) for item in command.get("stdout", "").split() if "=" in item)

    def prepare_and_publish(self) -> dict:
        destination = self.config.work_dir / "plugins" / self.plugin.package
        destination.mkdir(parents=True)
        for name in ("network-policy-dynamic.plugin.toml", "network-policy-dynamic.config.json",
                     "component-network-policy-dynamic.wasm", "config.schema.json"):
            shutil.copy2(self._package / name, destination / name)
        self.prepare()
        if self.current_config() != {"rules": []}:
            raise AssertionError("installed WASM publisher did not start with empty rules")
        loaded = self.loaded_plugin
        grants = loaded.get("host_grants", [])
        for rule in self._candidate["rules"]:
            expected = f"network-policy.rules.apply:kind={rule['decision']},remote={rule['remote']}"
            if expected not in grants:
                raise AssertionError(f"WASM publisher lacks grant: {expected}")
        before = [self._dry_run(rule["remote"]) for rule in self._candidate["rules"]]
        if any(fields.get("matched") != "false" or fields.get("decision") != "allow" for fields in before):
            raise AssertionError(f"empty static and WASM policies did not default allow: {before}")
        validation = self.api.validate_config(self.plugin.instance_id, self._candidate)
        if validation.get("valid") is not True:
            raise AssertionError(f"WASM policy validation failed: {validation}")
        started = time.perf_counter_ns()
        updated = self.api.update_config(self.plugin.instance_id, self._candidate)
        publish_ms = (time.perf_counter_ns() - started) / 1e6
        if updated.get("config") != self._candidate:
            raise AssertionError(f"WASM policy publication differs from requested rules: {updated}")
        after = [self._dry_run(rule["remote"]) for rule in self._candidate["rules"]]
        for old, fields, rule in zip(before, after, self._candidate["rules"]):
            expected = {"matched": "true", "decision": rule["decision"], "rule_id": rule["rule_id"],
                        "owner": self.plugin.instance_id, "remote": rule["remote"]}
            if any(fields.get(key) != value for key, value in expected.items()) or int(
                fields.get("source_revision", "0")
            ) <= int(old.get("source_revision", "0")):
                raise AssertionError(f"WASM publication is not active: {fields}")
        print(f"WASM policy active: update wall={publish_ms:.3f} ms; routes={after}", flush=True)
        return {"package": str(self._package), "loaded_plugin": loaded, "candidate": self._candidate,
                "validation": validation, "update": updated, "update_wall_ms": publish_ms,
                "before": before, "after": after, "static_rules": ""}


