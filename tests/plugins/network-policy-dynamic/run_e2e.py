#!/usr/bin/env python3
"""Run the Web-managed dynamic network policy E2E."""

from __future__ import annotations

import argparse
import importlib.util
import os
import shutil
import signal
import subprocess
import sys
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
FIXTURE_DIR = ROOT / "tests/plugins/network-policy-dynamic"
PLUGIN_SOURCE = ROOT / "examples/plugins/wit-component/network-policy-dynamic"
TIMEOUT_DIR = ROOT / "tests/plugins/control-timeout"
INSTANCE = "wasm.network-policy-dynamic"
REUSABLE_INSTANCE = "wasm.network-reusable-deny"
TIMEOUT_INSTANCE = "wasm.network-timeout"

sys.path.insert(0, str(ROOT))
from tests.v2.common.plugin_web_api import PluginWebApi  # noqa: E402
from scope_checks import NetworkPolicyScopeVerifier  # noqa: E402


def load_network_helpers():
    helper_path = ROOT / "tests/plugins/network-action/run_e2e.py"
    spec = importlib.util.spec_from_file_location("network_action_e2e", helper_path)
    if spec is None or spec.loader is None:
        raise RuntimeError(f"failed to load helper module {helper_path}")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


NETWORK = load_network_helpers()


class DynamicNetworkPolicyE2E:
    def __init__(self, args: argparse.Namespace, tmp: Path) -> None:
        self.args = args
        self.tmp = tmp
        self.bin_dir = ROOT / args.bin_dir
        self.actraild = NETWORK.require_binary(self.bin_dir, "actraild")
        self.actrailctl = NETWORK.require_binary(self.bin_dir, "actrailctl")
        self.actrailweb = NETWORK.require_binary(self.bin_dir, "actrailweb")
        self.actrailviewer = NETWORK.require_binary(self.bin_dir, "actrailviewer")
        self.rules = tmp / "network-rules.conf"
        self.config = tmp / "operator.conf"
        self.storage = tmp / "actrail.sqlite"
        self.plugin_root = tmp / "plugins"
        self.server = NETWORK.TcpProbeServer("dynamic-policy")
        self.secondary_server = NETWORK.TcpProbeServer("dynamic-policy-secondary")
        self.daemon: subprocess.Popen[str] | None = None
        self.web: subprocess.Popen[str] | None = None
        self.agent: subprocess.Popen[str] | None = None
        self.secondary_agent: subprocess.Popen[str] | None = None
        self.api: PluginWebApi | None = None

    @property
    def endpoint(self) -> str:
        return f"127.0.0.1:{self.server.port}"

    @property
    def secondary_endpoint(self) -> str:
        return f"127.0.0.1:{self.secondary_server.port}"

    @property
    def remote_scope(self) -> str:
        return "127.0.0.1:*"

    def run(self) -> None:
        self._prepare_files()
        self.server.start()
        self.secondary_server.start()
        self._start_daemon()
        self._load_decider(
            FIXTURE_DIR / "reusable-deny.plugin.toml",
            REUSABLE_INSTANCE,
            grants=("network-action.current-context-query",),
        )
        self._load_decider(
            TIMEOUT_DIR / "timeout.plugin.toml",
            TIMEOUT_INSTANCE,
            TIMEOUT_DIR / "timeout.config.toml",
        )
        self._start_web()
        self._load_publisher()
        assert self.api is not None
        scope_verifier = NetworkPolicyScopeVerifier(
            self.api, INSTANCE, self.endpoint, self.remote_scope
        )
        scope_verifier.require_grant_containment()
        self._load_publisher()
        scope_verifier.require_selector_validation()
        self._start_agents()
        self._require_atomic_rejection()
        self._require_overlap_rejection()
        self._require_live_local_updates()
        self._require_reusable_cache()
        self._require_timeout_and_overload()
        self._require_decider_unload_fails_closed()
        self._require_owner_unload_removes_policy()
        self._require_audit_evidence()
        expected = [b"live_allow", b"owner_unloaded"]
        if self.server.accepted != expected:
            raise AssertionError(
                f"server accepted unexpected governed connections: {self.server.accepted!r}"
            )
        if self.secondary_server.accepted != [b"live_allow_secondary"]:
            raise AssertionError(
                "secondary server accepted unexpected governed connections: "
                f"{self.secondary_server.accepted!r}"
            )

    def close(self) -> None:
        if self.agent is not None and self.agent.poll() is None:
            try:
                self._write_agent("quit")
                self.agent.wait(timeout=5)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self._stop_process(self.agent)
        if self.secondary_agent is not None and self.secondary_agent.poll() is None:
            try:
                self._write_agent("quit", self.secondary_agent)
                self.secondary_agent.wait(timeout=5)
            except (BrokenPipeError, subprocess.TimeoutExpired):
                self._stop_process(self.secondary_agent)
        if self.web is not None:
            self._stop_process(self.web)
        if self.daemon is not None:
            self._stop_process(self.daemon)
        self.server.close()
        self.secondary_server.close()

    def _prepare_files(self) -> None:
        package = self.plugin_root / "network-policy-dynamic"
        package.mkdir(parents=True)
        assets = {
            "plugin.toml": "network-policy-dynamic.plugin.toml",
            "network-policy-dynamic.config.json": "network-policy-dynamic.config.json",
            "component-network-policy-dynamic.wasm": "component-network-policy-dynamic.wasm",
            "config.schema.json": "config.schema.json",
        }
        for source_name, destination_name in assets.items():
            source = PLUGIN_SOURCE / source_name
            if not source.is_file():
                raise RuntimeError(f"official network-policy asset missing: {source}")
            shutil.copy2(source, package / destination_name)
        NETWORK.write_text(self.rules, "")
        base = NETWORK.operator_config(self.tmp, self.rules)
        NETWORK.write_text(
            self.config,
            base
            + "\n[plugins.discovery]\n"
            + f'directory = "{self.plugin_root}"\n'
            + "max_packages = 4\n"
            + "manifest_max_bytes = 262144\n"
            + "\n[plugins.startup]\n"
            + "enabled = false\n"
            + 'failure_policy = "fail-fast"\n',
        )
        NETWORK.write_text(self.tmp / "unused-enforcement-rules.conf", "")
        NETWORK.write_text(self.tmp / "unused-command-rules.conf", "")

    def _start_daemon(self) -> None:
        self.daemon = subprocess.Popen(
            [str(self.actraild), "--config", str(self.config), "run"],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        NETWORK.wait_for_daemon(self.daemon, self.args.daemon_ready_timeout_sec)

    def _start_web(self) -> None:
        self.web = subprocess.Popen(
            [
                str(self.actrailweb),
                "--config",
                str(self.config),
                "--addr",
                "127.0.0.1",
                "--port",
                "0",
            ],
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.STDOUT,
        )
        deadline = time.monotonic() + self.args.daemon_ready_timeout_sec
        while time.monotonic() < deadline:
            line = NETWORK.read_line_until(self.web, self.web.stdout, deadline)
            if line:
                print(line, end="")
                prefix = "actrailweb listening on "
                if line.startswith(prefix):
                    base_url = line.removeprefix(prefix).split()[0]
                    self.api = PluginWebApi(base_url, 10)
                    break
            if self.web.poll() is not None:
                raise RuntimeError("actrailweb exited before reporting readiness")
        if self.api is None:
            raise RuntimeError("actrailweb did not report readiness")
        for _ in range(self.args.drain_attempts):
            catalog = self.api.catalog()
            packages = catalog.get("packages")
            if isinstance(packages, list) and any(
                isinstance(item, dict)
                and item.get("package_key") == "network-policy-dynamic"
                and item.get("activation_ready") is True
                for item in packages
            ):
                return
            time.sleep(self.args.drain_sleep_sec)
        raise RuntimeError("network-policy-dynamic package did not become ready")

    def _load_decider(
        self,
        manifest: Path,
        instance: str,
        plugin_config: Path | None = None,
        grants: tuple[str, ...] = (),
    ) -> None:
        command = [
            str(self.actraild),
            "--config",
            str(self.config),
            "plugin",
            "load",
            "--manifest",
            str(manifest),
            "--instance",
            instance,
        ]
        if plugin_config is not None:
            command.extend(["--plugin-config", str(plugin_config)])
        for grant in grants:
            command.extend(["--grant", grant])
        NETWORK.run_checked(command)

    def _load_publisher(self) -> None:
        assert self.api is not None
        loaded = self.api.load(
            "network-policy-dynamic",
            INSTANCE,
            {
                "network_policy_rules_apply": [
                    {"decision": decision, "remote_scope": self.remote_scope}
                    for decision in ("allow", "deny", "gray")
                ]
            },
        )
        plugin = loaded.get("plugin")
        if not isinstance(plugin, dict) or plugin.get("state") != "active":
            raise AssertionError(f"publisher load failed: {loaded}")
        grants = plugin.get("host_grants")
        expected = (
            "network-policy.rules.apply:kind=deny,"
            f"remote={self.remote_scope}"
        )
        if not isinstance(grants, list) or expected not in grants:
            raise AssertionError(f"publisher missed scoped network grant: {grants}")

    def _start_agents(self) -> None:
        self.agent = self._launch_agent("dynamic-network-policy-primary")
        self.secondary_agent = self._launch_agent("dynamic-network-policy-secondary")

    def _launch_agent(self, trace_name: str) -> subprocess.Popen[str]:
        agent = subprocess.Popen(
            [
                str(self.actrailctl),
                "--config",
                str(self.config),
                "launch",
                "--name",
                trace_name,
                "--",
                sys.executable,
                str(FIXTURE_DIR / "agent.py"),
            ],
            text=True,
            stdin=subprocess.PIPE,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
        )
        NETWORK.wait_for_agent_pid(agent, self.args.agent_timeout_sec)
        return agent

    def _require_atomic_rejection(self) -> None:
        assert self.api is not None
        before = self._source_revision()
        initial = self.api.config(INSTANCE).get("config")
        candidate = {
            "rules": [
                {
                    "rule_id": "atomic-granted",
                    "decision": "deny",
                    "remote": self.remote_scope,
                },
                {
                    "rule_id": "atomic-ungranted",
                    "decision": "deny",
                    "remote": "127.0.0.2:*",
                },
            ]
        }
        validation = self.api.validate_config(INSTANCE, candidate)
        errors = validation.get("errors")
        if validation.get("valid") is not False or not isinstance(errors, list):
            raise AssertionError(f"out-of-grant candidate was accepted: {validation}")
        if not any("missing network-policy.rules.apply grant" in str(error) for error in errors):
            raise AssertionError(f"grant rejection reason is missing: {validation}")
        if self.api.config(INSTANCE).get("config") != initial:
            raise AssertionError("failed network Configuration Test changed plugin memory")
        if self._source_revision() != before:
            raise AssertionError("failed network Configuration Test changed daemon revision")

    def _require_overlap_rejection(self) -> None:
        assert self.api is not None
        before = self._source_revision()
        initial = self.api.config(INSTANCE).get("config")
        candidate = {
            "rules": [
                {
                    "rule_id": "overlap-all-ports",
                    "decision": "deny",
                    "remote": self.remote_scope,
                },
                {
                    "rule_id": "overlap-exact-port",
                    "decision": "allow",
                    "remote": self.endpoint,
                },
            ]
        }
        validation = self.api.validate_config(INSTANCE, candidate)
        errors = validation.get("errors")
        if validation.get("valid") is not False or not isinstance(errors, list):
            raise AssertionError(f"overlapping network selectors were accepted: {validation}")
        if not any("overlap" in str(error) for error in errors):
            raise AssertionError(f"overlap rejection reason is missing: {validation}")
        if self.api.config(INSTANCE).get("config") != initial:
            raise AssertionError("overlap validation changed plugin memory")
        if self._source_revision() != before:
            raise AssertionError("overlap validation changed daemon revision")

    def _require_live_local_updates(self) -> None:
        self._update_rule("deny")
        self._connect("live_deny", "permission_denied")
        self._connect(
            "live_deny_secondary", "permission_denied", self.secondary_server
        )
        self._update_rule("allow")
        self._connect("live_allow", "ok")
        self._connect("live_allow_secondary", "ok", self.secondary_server)

    def _require_reusable_cache(self) -> None:
        self._update_rule(
            "gray",
            gray_target=REUSABLE_INSTANCE,
            timeout_ms=1000,
            concurrency_limit=1,
            fallback="deny",
        )
        self._connect("reusable_first", "permission_denied")
        self._wait_for_metadata("gray-plugin")
        self._connect("reusable_cached", "permission_denied")
        self._wait_for_metadata("gray-plugin-cache")
        status = NETWORK.parse_status_fields(
            NETWORK.run_checked(
                [
                    str(self.actraild),
                    "--config",
                    str(self.config),
                    "plugin",
                    "status",
                    "--instance",
                    REUSABLE_INSTANCE,
                ]
            )
        )
        if status.get("observed_records") != "1":
            raise AssertionError(f"reusable decision was not cached: {status}")
        if status.get("host_grants") != "network-action.current-context-query":
            raise AssertionError(f"typed network context grant is missing: {status}")

    def _require_timeout_and_overload(self) -> None:
        self._update_rule(
            "gray",
            gray_target=TIMEOUT_INSTANCE,
            timeout_ms=150,
            concurrency_limit=1,
            fallback="deny",
        )
        if self.secondary_agent is None:
            raise RuntimeError("secondary agent is unavailable")
        self._write_agent(f"connect {self.server.port} overloaded_primary")
        self._write_agent(
            f"connect {self.server.port} overloaded_secondary", self.secondary_agent
        )
        self._wait_agent_markers({"overloaded_primary=permission_denied"})
        self._wait_agent_markers(
            {"overloaded_secondary=permission_denied"}, self.secondary_agent
        )
        self._wait_for_metadata("fallback", "plugin_timeout")
        self._wait_for_metadata("fallback", "rule_concurrency_limit")

    def _require_decider_unload_fails_closed(self) -> None:
        self._update_rule(
            "gray",
            gray_target=REUSABLE_INSTANCE,
            timeout_ms=1000,
            concurrency_limit=1,
            fallback="allow",
        )
        NETWORK.run_checked(
            [
                str(self.actraild),
                "--config",
                str(self.config),
                "plugin",
                "unload",
                "--instance",
                REUSABLE_INSTANCE,
            ]
        )
        self._connect("decider_unloaded", "permission_denied")
        self._wait_for_metadata("fallback", "plugin_unloaded")

    def _require_owner_unload_removes_policy(self) -> None:
        assert self.api is not None
        unloaded = self.api.unload(INSTANCE)
        plugin = unloaded.get("plugin")
        if not isinstance(plugin, dict) or plugin.get("state") == "active":
            raise AssertionError(f"publisher remained active: {unloaded}")
        self._connect("owner_unloaded", "ok")

    def _require_audit_evidence(self) -> None:
        expectations = [
            ("rule", None, "deny"),
            ("rule", None, "allow"),
            ("gray-plugin", None, "deny"),
            ("gray-plugin-cache", None, "deny"),
            ("fallback", "plugin_timeout", "deny"),
            ("fallback", "rule_concurrency_limit", "deny"),
            ("fallback", "plugin_unloaded", "deny"),
        ]
        metadata = self._all_metadata()
        governed = [item for item in metadata if "rule_id" in item]
        if not governed or any(
            item.get("policy_remote_scope") != self.remote_scope for item in governed
        ):
            raise AssertionError(
                f"network audit missed wildcard policy scope {self.remote_scope}: {governed}"
            )
        for source, reason, decision in expectations:
            if not any(
                item.get("decision_source") == source
                and item.get("decision") == decision
                and (reason is None or item.get("fallback_reason") == reason)
                for item in metadata
            ):
                raise AssertionError(
                    f"network audit missed source={source} reason={reason}: {metadata}"
                )

    def _update_rule(self, decision: str, **gray: object) -> None:
        assert self.api is not None
        rule: dict[str, object] = {
            "rule_id": "managed-endpoint",
            "decision": decision,
            "remote": self.remote_scope,
        }
        rule.update(gray)
        candidate = {"rules": [rule]}
        validation = self.api.validate_config(INSTANCE, candidate)
        if validation.get("valid") is not True:
            raise AssertionError(f"network rule validation failed: {validation}")
        updated = self.api.update_config(INSTANCE, candidate)
        if updated.get("config") != candidate:
            raise AssertionError(f"network rule update changed candidate: {updated}")
        dry_run = self._plugin_command(["rule", "dry-run", self.endpoint])
        if f"decision={decision}" not in dry_run or f"owner={INSTANCE}" not in dry_run:
            raise AssertionError(f"published network rule did not match: {dry_run}")
        secondary_dry_run = self._plugin_command(
            ["rule", "dry-run", self.secondary_endpoint]
        )
        if (
            f"decision={decision}" not in secondary_dry_run
            or f"owner={INSTANCE}" not in secondary_dry_run
        ):
            raise AssertionError(
                f"published any-port rule missed secondary endpoint: {secondary_dry_run}"
            )

    def _source_revision(self) -> str:
        output = self._plugin_command(["rule", "dry-run", self.endpoint])
        for item in output.split():
            if item.startswith("source_revision="):
                return item.split("=", 1)[1]
        raise AssertionError(f"network dry-run omitted source revision: {output}")

    def _plugin_command(self, argv: list[str]) -> str:
        assert self.api is not None
        response = self.api.command(INSTANCE, argv)
        command = response.get("command")
        if not isinstance(command, dict) or command.get("exit_code") != 0:
            raise AssertionError(f"plugin command failed: {response}")
        return str(command.get("stdout", ""))

    def _connect(
        self,
        label: str,
        expected: str,
        server: object | None = None,
    ) -> None:
        target = server or self.server
        self._write_agent(f"connect {target.port} {label}")
        self._wait_agent_markers({f"{label}={expected}"})

    def _write_agent(
        self, command: str, agent: subprocess.Popen[str] | None = None
    ) -> None:
        target = agent or self.agent
        if target is None or target.stdin is None:
            raise RuntimeError("agent stdin is unavailable")
        target.stdin.write(command + "\n")
        target.stdin.flush()

    def _wait_agent_markers(
        self, markers: set[str], agent: subprocess.Popen[str] | None = None
    ) -> None:
        target = agent or self.agent
        if target is None or target.stdout is None:
            raise RuntimeError("agent stdout is unavailable")
        pending = set(markers)
        deadline = time.monotonic() + self.args.agent_timeout_sec
        while pending and time.monotonic() < deadline:
            line = NETWORK.read_line_until(target, target.stdout, deadline)
            if line:
                print(line, end="")
                pending.discard(line.strip())
            if target.poll() is not None:
                stderr = target.stderr.read() if target.stderr else ""
                raise RuntimeError(f"agent exited early: {stderr}")
        if pending:
            raise RuntimeError(f"agent did not report markers: {sorted(pending)}")

    def _wait_for_metadata(self, source: str, reason: str | None = None) -> None:
        for _ in range(self.args.drain_attempts):
            if any(
                item.get("decision_source") == source
                and (reason is None or item.get("fallback_reason") == reason)
                for item in self._all_metadata()
            ):
                return
            time.sleep(self.args.drain_sleep_sec)
        raise AssertionError(
            f"network audit missed source={source} reason={reason}: {self._all_metadata()}"
        )

    def _all_metadata(self) -> list[dict[str, str]]:
        NETWORK.run_checked(
            [str(self.actrailctl), "--config", str(self.config), "list-traces"]
        )
        metadata: list[dict[str, str]] = []
        for trace_id in (1, 2):
            snapshot = NETWORK.EventSnapshot.load(
                self.actrailviewer, self.config, trace_id
            )
            for event in snapshot.events("net"):
                fields = event.payload
                if fields.get("transport") != "inet" or fields.get("remote") not in {
                    self.endpoint,
                    self.secondary_endpoint,
                }:
                    continue
                event_metadata = fields.get("metadata")
                if isinstance(event_metadata, dict):
                    metadata.append(
                        {
                            str(key): str(value)
                            for key, value in event_metadata.items()
                        }
                    )
        return metadata

    @staticmethod
    def _stop_process(process: subprocess.Popen[str]) -> None:
        if process.poll() is not None:
            return
        process.send_signal(signal.SIGTERM)
        try:
            process.wait(timeout=5)
        except subprocess.TimeoutExpired:
            process.kill()
            process.wait()


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser()
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--daemon-ready-timeout-sec", type=float, default=10.0)
    parser.add_argument("--agent-timeout-sec", type=float, default=10.0)
    parser.add_argument("--drain-attempts", type=int, default=30)
    parser.add_argument("--drain-sleep-sec", type=float, default=0.2)
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    NETWORK.require_root()
    temp_root = ROOT / "temp"
    temp_root.mkdir(exist_ok=True)
    import tempfile

    with tempfile.TemporaryDirectory(
        prefix="network-policy-dynamic-e2e-", dir=temp_root
    ) as raw_tmp:
        test = DynamicNetworkPolicyE2E(args, Path(raw_tmp))
        try:
            test.run()
        finally:
            test.close()
    print("network_policy_dynamic_e2e=ok")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
