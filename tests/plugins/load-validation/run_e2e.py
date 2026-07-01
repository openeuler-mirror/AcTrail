#!/usr/bin/env python3
"""Run plugin load validation E2E checks."""

from __future__ import annotations

import os
import shutil
import signal
import subprocess
import time
from pathlib import Path


ROOT = Path(__file__).resolve().parents[3]
RUN_DIR = Path("/tmp/actrail-plugin-load-validation")
SOURCE_CONFIG = ROOT / "tests/plugins/otel-jsonl/operator.conf"
CONFIG = RUN_DIR / "operator.conf"
REGISTRY = RUN_DIR / "operator.conf.plugins.toml"
SOCKET_PATH = RUN_DIR / "actraild.sock"
REQUIRED_CONFIG_MANIFEST = ROOT / "tests/plugins/wasm-observation/count.plugin.toml"
BAD_TOML_CONFIG = ROOT / "tests/plugins/load-validation/bad.config.toml"
UNSUPPORTED_FORMAT_MANIFEST = ROOT / "tests/plugins/load-validation/unsupported-format.plugin.toml"
UNSUPPORTED_FORMAT_CONFIG = ROOT / "tests/plugins/load-validation/unsupported-format.config.txt"
EMPTY_ID_MANIFEST = ROOT / "tests/plugins/load-validation/empty-id.plugin.toml"
UNSUPPORTED_API_MANIFEST = ROOT / "tests/plugins/load-validation/unsupported-api.plugin.toml"
SCHEMA_REQUIRED_MANIFEST = ROOT / "tests/plugins/load-validation/schema-required.plugin.toml"
SCHEMA_INVALID_CONFIG = ROOT / "tests/plugins/load-validation/schema-invalid.config.toml"
SCHEMA_VALID_CONFIG = ROOT / "tests/plugins/load-validation/schema-valid.config.toml"
MISSING_SCHEMA_MANIFEST = ROOT / "tests/plugins/load-validation/missing-schema.plugin.toml"
UNGRANTED_CAPABILITY_MANIFEST = ROOT / "tests/plugins/load-validation/ungranted-capability.plugin.toml"
ZERO_CONTROL_CONCURRENCY_MANIFEST = ROOT / "tests/plugins/load-validation/zero-control-concurrency.plugin.toml"
ZERO_PAYLOAD_REF_MANIFEST = ROOT / "tests/plugins/load-validation/zero-payload-ref.plugin.toml"
ZERO_OBSERVATION_QUEUE_MANIFEST = ROOT / "tests/plugins/load-validation/zero-observation-queue.plugin.toml"
ZERO_PLUGIN_CONFIG_READ_MANIFEST = RUN_DIR / "zero-plugin-config-read.plugin.toml"
ZERO_PLUGIN_COMMAND_TIMEOUT_MANIFEST = RUN_DIR / "zero-plugin-command-timeout.plugin.toml"
EMPTY_EVENT_FAMILIES_MANIFEST = ROOT / "tests/plugins/load-validation/empty-event-families.plugin.toml"
CONTROL_EVENT_FAMILIES_MANIFEST = ROOT / "tests/plugins/load-validation/control-event-families.plugin.toml"
FILE_ACCESS_CURRENT_MATCH_GET_MANIFEST = ROOT / "tests/plugins/control-file-policy-read/policy-read.plugin.toml"
ZERO_FILE_ACCESS_CURRENT_MATCH_GET_MANIFEST = ROOT / "tests/plugins/load-validation/zero-file-policy-read.plugin.toml"
CONTEXT_QUERY_MANIFEST = ROOT / "tests/plugins/control-context-query/context-query.plugin.toml"
ZERO_CONTEXT_QUERY_MANIFEST = ROOT / "tests/plugins/load-validation/zero-context-query.plugin.toml"
NATIVE_OBSERVATION_MANIFEST = ROOT / "tests/plugins/load-validation/native-observation.plugin.toml"
NATIVE_CONTROL_MANIFEST = ROOT / "tests/plugins/load-validation/native-control.plugin.toml"
NETWORK_EGRESS_MANIFEST = RUN_DIR / "network-egress.plugin.toml"
BUILTIN_HOST_CAPABILITY_MANIFEST = RUN_DIR / "builtin-host-capability.plugin.toml"
BUILTIN_HOST_CAPABILITY_CONFIG = RUN_DIR / "builtin-host-capability.config.toml"
BUILTIN_ARTIFACT_PATH_MANIFEST = RUN_DIR / "builtin-artifact-path.plugin.toml"
BUILTIN_ARTIFACT_PATH_CONFIG = RUN_DIR / "builtin-artifact-path.config.toml"
DUPLICATE_EVENT_FAMILIES_MANIFEST = RUN_DIR / "duplicate-event-families.plugin.toml"
DUPLICATE_CAPABILITIES_MANIFEST = RUN_DIR / "duplicate-capabilities.plugin.toml"
BUILTIN_WASM_ABI_MANIFEST = RUN_DIR / "builtin-wasm-abi.plugin.toml"
MISSING_WASM_ARTIFACT_MANIFEST = RUN_DIR / "missing-wasm-artifact.plugin.toml"
WHITESPACE_WASM_ARTIFACT_MANIFEST = RUN_DIR / "whitespace-wasm-artifact.plugin.toml"
EMPTY_SCHEMA_REF_MANIFEST = RUN_DIR / "empty-schema-ref.plugin.toml"
MISSING_CONFIG_INSTANCE = "wasm.missing-required-config"
BAD_TOML_INSTANCE = "wasm.invalid-toml-config"
UNSUPPORTED_FORMAT_INSTANCE = "wasm.unsupported-config-format"
EMPTY_ID_INSTANCE = "wasm.empty-id"
UNSUPPORTED_API_INSTANCE = "wasm.unsupported-api"
EMPTY_SCHEMA_REF_INSTANCE = "wasm.empty-schema-ref"
SCHEMA_INVALID_INSTANCE = "wasm.schema-invalid-config"
SCHEMA_VALID_INSTANCE = "wasm.schema-valid-config"
EMPTY_INSTANCE_ID = ""
WHITESPACE_INSTANCE_ID = "   "
TLS_SOURCE_GRANT_INSTANCE = "wasm.tls-source-grant"
MISSING_SCHEMA_INSTANCE = "wasm.missing-schema"
UNGRANTED_CAPABILITY_INSTANCE = "wasm.ungranted-capability"
DUPLICATE_HOST_GRANT_INSTANCE = "wasm.duplicate-host-grant"
ZERO_CONTROL_CONCURRENCY_INSTANCE = "wasm.zero-control-concurrency"
ZERO_PAYLOAD_REF_INSTANCE = "wasm.zero-payload-ref"
ZERO_OBSERVATION_QUEUE_INSTANCE = "wasm.zero-observation-queue"
ZERO_PLUGIN_CONFIG_READ_INSTANCE = "wasm.zero-plugin-config-read"
ZERO_PLUGIN_COMMAND_TIMEOUT_INSTANCE = "wasm.zero-plugin-command-timeout"
EMPTY_EVENT_FAMILIES_INSTANCE = "wasm.empty-event-families"
CONTROL_EVENT_FAMILIES_INSTANCE = "wasm.control-event-families"
DUPLICATE_EVENT_FAMILIES_INSTANCE = "wasm.duplicate-event-families"
DUPLICATE_CAPABILITIES_INSTANCE = "wasm.duplicate-capabilities"
BUILTIN_WASM_ABI_INSTANCE = "builtin.invalid-wasm-abi"
MISSING_WASM_ARTIFACT_INSTANCE = "wasm.missing-artifact"
WHITESPACE_WASM_ARTIFACT_INSTANCE = "wasm.whitespace-artifact"
UNGRANTED_FILE_ACCESS_CURRENT_MATCH_GET_INSTANCE = "wasm.ungranted-file-access.current-match-get"
ZERO_FILE_ACCESS_CURRENT_MATCH_GET_INSTANCE = "wasm.zero-file-access.current-match-get"
UNGRANTED_CONTEXT_QUERY_INSTANCE = "wasm.ungranted-context-query"
ZERO_CONTEXT_QUERY_INSTANCE = "wasm.zero-context-query"
NATIVE_OBSERVATION_INSTANCE = "native.observation-disabled"
NATIVE_CONTROL_INSTANCE = "native.control-disabled"
UNGRANTED_NETWORK_EGRESS_INSTANCE = "wasm.ungranted-network-egress"
UNSUPPORTED_NETWORK_EGRESS_GRANT_INSTANCE = "wasm.unsupported-network-egress-grant"
BUILTIN_HOST_CAPABILITY_INSTANCE = "builtin.otel-jsonl-host-capability"
BUILTIN_ARTIFACT_PATH_INSTANCE = "builtin.otel-jsonl-artifact-path"


def run(cmd: list[str], *, timeout: int = 60, check: bool = True) -> subprocess.CompletedProcess[str]:
    completed = subprocess.run(
        cmd,
        cwd=ROOT,
        text=True,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        timeout=timeout,
    )
    if check and completed.returncode != 0:
        raise RuntimeError(f"command failed {cmd}: exit={completed.returncode}\n{completed.stdout[-4000:]}")
    return completed


def wait_for_socket(path: Path, timeout: float = 10.0) -> None:
    deadline = time.time() + timeout
    while time.time() < deadline:
        if path.exists():
            return
        time.sleep(0.05)
    raise RuntimeError(f"daemon socket did not appear: {path}")


def assert_registry_absent() -> None:
    if REGISTRY.exists():
        registry_raw = REGISTRY.read_text(encoding="utf-8")
        raise RuntimeError(f"persistent plugin registry should not exist\n{registry_raw}")


def write_config() -> None:
    raw = SOURCE_CONFIG.read_text(encoding="utf-8")
    raw = raw.replace("/tmp/actrail-plugin-otel-jsonl", str(RUN_DIR))
    raw = raw.replace("[export.runtime]\nenabled = true", "[export.runtime]\nenabled = false")
    raw = raw.replace("[plugins.startup]\nenabled = true", "[plugins.startup]\nenabled = false")
    CONFIG.write_text(raw, encoding="utf-8")


def write_dynamic_validation_inputs() -> None:
    BUILTIN_HOST_CAPABILITY_MANIFEST.write_text(
        """[general]
id = "otel-jsonl"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "builtin"

[host]
capabilities = ["payload-read"]

[plugin_config]
format = "toml"
required = true
""",
        encoding="utf-8",
    )
    BUILTIN_HOST_CAPABILITY_CONFIG.write_text(
        "\n".join(
            [
                f'path = "{RUN_DIR / "builtin-host-capability.otlp.jsonl"}"',
                "overwrite_enabled = true",
                "queue_capacity = 128",
                "flush_every_spans = 1",
                "",
            ]
        ),
        encoding="utf-8",
    )
    BUILTIN_ARTIFACT_PATH_MANIFEST.write_text(
        """[general]
id = "otel-jsonl"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "builtin"

[host]
capabilities = []

[runtime.wasm]
artifact_path = "unused-builtin-plugin.wasm"

[plugin_config]
format = "toml"
required = true
""",
        encoding="utf-8",
    )
    BUILTIN_ARTIFACT_PATH_CONFIG.write_text(
        "\n".join(
            [
                f'path = "{RUN_DIR / "builtin-artifact-path.otlp.jsonl"}"',
                "overwrite_enabled = true",
                "queue_capacity = 128",
                "flush_every_spans = 1",
                "",
            ]
        ),
        encoding="utf-8",
    )
    NETWORK_EGRESS_MANIFEST.write_text(
        f"""[general]
id = "wasm.future-network-egress"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = ["network-egress"]

[runtime.wasm]
artifact_path = "{ROOT / "tests/plugins/wasm-observation/count.wat"}"

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    ZERO_PLUGIN_CONFIG_READ_MANIFEST.write_text(
        f"""[general]
id = "wasm.zero-plugin-config-read"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = []

[runtime.wasm]
artifact_path = "{ROOT / "tests/plugins/wasm-component-observation/component-config.wasm"}"
abi = "wit-component"

[hostcall_limits.plugin_config]
read_max_bytes = 0

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    ZERO_PLUGIN_COMMAND_TIMEOUT_MANIFEST.write_text(
        f"""[general]
id = "wasm.zero-plugin-command-timeout"
api_version = "actrail.plugin.v1"
role = "control-decider"
runtime = "wasm"

[host]
capabilities = []

[runtime.wasm]
artifact_path = "{ROOT / "tests/plugins/wasm-component-control-graylist/component-allow.wasm"}"
abi = "wit-component"

[hostcall_limits.plugin_command]
timeout_ms = 0

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    DUPLICATE_EVENT_FAMILIES_MANIFEST.write_text(
        f"""[general]
id = "wasm.duplicate-event-families"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = []

[runtime.wasm]
artifact_path = "{ROOT / "tests/plugins/wasm-observation/count.wat"}"

[role.observation-consumer.subscriptions]
event_families = ["semantic-action", "semantic-action"]

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    DUPLICATE_CAPABILITIES_MANIFEST.write_text(
        f"""[general]
id = "wasm.duplicate-capabilities"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = ["payload-read", "payload-read"]

[runtime.wasm]
artifact_path = "{ROOT / "tests/plugins/wasm-observation/count.wat"}"

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    BUILTIN_WASM_ABI_MANIFEST.write_text(
        """[general]
id = "otel-jsonl"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "builtin"

[host]
capabilities = []

[runtime.wasm]
abi = "wit-component"

[plugin_config]
format = "toml"
required = false
""",
        encoding="utf-8",
    )
    MISSING_WASM_ARTIFACT_MANIFEST.write_text(
        """[general]
id = "wasm.missing-artifact"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = []

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    WHITESPACE_WASM_ARTIFACT_MANIFEST.write_text(
        """[general]
id = "wasm.whitespace-artifact"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = []

[runtime.wasm]
artifact_path = "   "

[plugin_config]
format = "toml"
schema_ref = ""
required = false
""",
        encoding="utf-8",
    )
    EMPTY_SCHEMA_REF_MANIFEST.write_text(
        f"""[general]
id = "wasm.empty-schema-ref"
api_version = "actrail.plugin.v1"
role = "observation-consumer"
runtime = "wasm"

[host]
capabilities = []

[runtime.wasm]
artifact_path = "{ROOT / "tests/plugins/wasm-observation/count.wat"}"

[plugin_config]
format = "toml"
schema_ref = "   "
required = true
""",
        encoding="utf-8",
    )


def assert_load_fails(
    actraild: Path,
    manifest: Path,
    instance: str,
    expected: list[str],
    plugin_config: Path | None = None,
    host_grants: list[str] | None = None,
    persist: bool = False,
) -> None:
    command = [
        str(actraild),
        "--config",
        str(CONFIG),
        "plugin",
        "load",
        "--manifest",
        str(manifest),
    ]
    if plugin_config is not None:
        command.extend(["--plugin-config", str(plugin_config)])
    for grant in host_grants or []:
        command.extend(["--grant", grant])
    if persist:
        command.append("--persist")
    command.extend(["--instance", instance])
    load = run(command, check=False)
    if load.returncode == 0:
        raise RuntimeError(f"plugin load unexpectedly succeeded for {instance}")
    missing = [value for value in expected if value not in load.stdout]
    if missing:
        raise RuntimeError(
            f"plugin load error for {instance} missed {missing}\n{load.stdout}"
        )


def assert_load_succeeds(
    actraild: Path,
    manifest: Path,
    instance: str,
    plugin_config: Path,
    host_grants: list[str] | None = None,
) -> None:
    command = [
        str(actraild),
        "--config",
        str(CONFIG),
        "plugin",
        "load",
        "--manifest",
        str(manifest),
        "--plugin-config",
        str(plugin_config),
    ]
    for grant in host_grants or []:
        command.extend(["--grant", grant])
    command.extend(["--instance", instance])
    load = run(command)
    if f"loaded instance={instance}" not in load.stdout:
        raise RuntimeError(f"plugin load did not report loaded instance {instance}\n{load.stdout}")


def assert_unload_fails(actraild: Path, instance: str, expected: list[str]) -> None:
    unload = run(
        [
            str(actraild),
            "--config",
            str(CONFIG),
            "plugin",
            "unload",
            "--instance",
            instance,
        ],
        check=False,
    )
    if unload.returncode == 0:
        raise RuntimeError(f"plugin unload unexpectedly succeeded for {instance}")
    missing = [value for value in expected if value not in unload.stdout]
    if missing:
        raise RuntimeError(
            f"plugin unload error for {instance} missed {missing}\n{unload.stdout}"
        )


def assert_status_fails(actraild: Path, instance: str, expected: list[str]) -> None:
    status = run(
        [
            str(actraild),
            "--config",
            str(CONFIG),
            "plugin",
            "status",
            "--instance",
            instance,
        ],
        check=False,
    )
    if status.returncode == 0:
        raise RuntimeError(f"plugin status unexpectedly succeeded for {instance}")
    missing = [value for value in expected if value not in status.stdout]
    if missing:
        raise RuntimeError(
            f"plugin status error for {instance} missed {missing}\n{status.stdout}"
        )


def main() -> int:
    bin_dir = Path(os.environ.get("ACTRAIL_BIN_DIR", ROOT / "target/release"))
    actraild = bin_dir / "actraild"

    if RUN_DIR.exists():
        shutil.rmtree(RUN_DIR)
    RUN_DIR.mkdir(parents=True)
    write_config()
    write_dynamic_validation_inputs()

    daemon = subprocess.Popen(
        [str(actraild), "--config", str(CONFIG), "run"],
        cwd=ROOT,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    try:
        wait_for_socket(SOCKET_PATH)
        assert_load_fails(
            actraild,
            REQUIRED_CONFIG_MANIFEST,
            MISSING_CONFIG_INSTANCE,
            ["plugin_config", "requires"],
        )
        assert_load_fails(
            actraild,
            REQUIRED_CONFIG_MANIFEST,
            BAD_TOML_INSTANCE,
            ["plugin_config", "parse TOML"],
            BAD_TOML_CONFIG,
        )
        assert_load_fails(
            actraild,
            UNSUPPORTED_FORMAT_MANIFEST,
            UNSUPPORTED_FORMAT_INSTANCE,
            ["plugin_config", "unsupported format"],
            UNSUPPORTED_FORMAT_CONFIG,
        )
        assert_load_fails(
            actraild,
            EMPTY_ID_MANIFEST,
            EMPTY_ID_INSTANCE,
            ["plugin_manifest", "id"],
            ROOT / "tests/plugins/wasm-observation/count.config.toml",
        )
        assert_load_fails(
            actraild,
            UNSUPPORTED_API_MANIFEST,
            UNSUPPORTED_API_INSTANCE,
            ["plugin_manifest", "api_version"],
            ROOT / "tests/plugins/wasm-observation/count.config.toml",
        )
        assert_load_fails(
            actraild,
            MISSING_SCHEMA_MANIFEST,
            MISSING_SCHEMA_INSTANCE,
            ["plugin_config", "schema_ref"],
            SCHEMA_VALID_CONFIG,
        )
        assert_load_fails(
            actraild,
            EMPTY_SCHEMA_REF_MANIFEST,
            EMPTY_SCHEMA_REF_INSTANCE,
            ["plugin_config", "schema_ref", "must not be empty"],
            SCHEMA_VALID_CONFIG,
        )
        assert_load_fails(
            actraild,
            SCHEMA_REQUIRED_MANIFEST,
            SCHEMA_INVALID_INSTANCE,
            ["plugin_config", "schema"],
            SCHEMA_INVALID_CONFIG,
        )
        assert_load_succeeds(
            actraild,
            SCHEMA_REQUIRED_MANIFEST,
            SCHEMA_VALID_INSTANCE,
            SCHEMA_VALID_CONFIG,
        )
        assert_load_fails(
            actraild,
            SCHEMA_REQUIRED_MANIFEST,
            EMPTY_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
            SCHEMA_VALID_CONFIG,
        )
        assert_load_fails(
            actraild,
            SCHEMA_REQUIRED_MANIFEST,
            EMPTY_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
            SCHEMA_VALID_CONFIG,
            persist=True,
        )
        assert_registry_absent()
        assert_unload_fails(
            actraild,
            EMPTY_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
        )
        assert_status_fails(
            actraild,
            EMPTY_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
        )
        assert_load_fails(
            actraild,
            SCHEMA_REQUIRED_MANIFEST,
            WHITESPACE_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
            SCHEMA_VALID_CONFIG,
        )
        assert_load_fails(
            actraild,
            SCHEMA_REQUIRED_MANIFEST,
            WHITESPACE_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
            SCHEMA_VALID_CONFIG,
            persist=True,
        )
        assert_registry_absent()
        assert_unload_fails(
            actraild,
            WHITESPACE_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
        )
        assert_status_fails(
            actraild,
            WHITESPACE_INSTANCE_ID,
            ["plugin_command", "plugin instance id must not be empty"],
        )
        assert_load_succeeds(
            actraild,
            UNGRANTED_CAPABILITY_MANIFEST,
            TLS_SOURCE_GRANT_INSTANCE,
            SCHEMA_VALID_CONFIG,
            ["payload-read:source=tls-user-space"],
        )
        assert_load_fails(
            actraild,
            UNGRANTED_CAPABILITY_MANIFEST,
            UNGRANTED_CAPABILITY_INSTANCE,
            ["plugin_capability", "payload-read"],
            SCHEMA_VALID_CONFIG,
        )
        assert_load_fails(
            actraild,
            UNGRANTED_CAPABILITY_MANIFEST,
            f"{UNGRANTED_CAPABILITY_INSTANCE}.bad-source",
            ["plugin_capability", "unsupported payload-read source", "bogus"],
            host_grants=["payload-read:source=bogus"],
        )
        assert_load_fails(
            actraild,
            UNGRANTED_CAPABILITY_MANIFEST,
            f"{UNGRANTED_CAPABILITY_INSTANCE}.mixed-payload-grants",
            ["plugin_capability", "cannot combine payload-read"],
            host_grants=["payload-read", "payload-read:source=syscall"],
        )
        assert_load_fails(
            actraild,
            UNGRANTED_CAPABILITY_MANIFEST,
            DUPLICATE_HOST_GRANT_INSTANCE,
            ["plugin_capability", "duplicate plugin host grant", "payload-read"],
            SCHEMA_VALID_CONFIG,
            host_grants=["payload-read", "payload-read"],
        )
        assert_load_fails(
            actraild,
            REQUIRED_CONFIG_MANIFEST,
            f"{MISSING_CONFIG_INSTANCE}.unexpected-payload-source-grant",
            ["plugin_capability", "payload-read:source=syscall", "did not request"],
            host_grants=["payload-read:source=syscall"],
        )
        assert_load_fails(
            actraild,
            ZERO_CONTROL_CONCURRENCY_MANIFEST,
            ZERO_CONTROL_CONCURRENCY_INSTANCE,
            ["plugin_manifest", "role.control-decider.resources.concurrency_limit", "greater than zero"],
            ROOT / "tests/plugins/control-timeout/timeout.config.toml",
        )
        assert_load_fails(
            actraild,
            ZERO_PAYLOAD_REF_MANIFEST,
            ZERO_PAYLOAD_REF_INSTANCE,
            ["plugin_manifest", "hostcall_limits.payload.ref_max_bytes", "greater than zero"],
        )
        assert_load_fails(
            actraild,
            ZERO_OBSERVATION_QUEUE_MANIFEST,
            ZERO_OBSERVATION_QUEUE_INSTANCE,
            ["plugin_manifest", "role.observation-consumer.resources.queue_capacity", "greater than zero"],
        )
        assert_load_fails(
            actraild,
            ZERO_PLUGIN_CONFIG_READ_MANIFEST,
            ZERO_PLUGIN_CONFIG_READ_INSTANCE,
            [
                "plugin_manifest",
                "hostcall_limits.plugin_config.read_max_bytes",
                "greater than zero",
            ],
        )
        assert_load_fails(
            actraild,
            ZERO_PLUGIN_COMMAND_TIMEOUT_MANIFEST,
            ZERO_PLUGIN_COMMAND_TIMEOUT_INSTANCE,
            [
                "plugin_manifest",
                "hostcall_limits.plugin_command.timeout_ms",
                "greater than zero",
            ],
        )
        assert_load_fails(
            actraild,
            EMPTY_EVENT_FAMILIES_MANIFEST,
            EMPTY_EVENT_FAMILIES_INSTANCE,
            ["plugin_manifest", "subscriptions.event_families", "must not be empty"],
        )
        assert_load_fails(
            actraild,
            CONTROL_EVENT_FAMILIES_MANIFEST,
            CONTROL_EVENT_FAMILIES_INSTANCE,
            ["plugin_manifest", "role.observation-consumer", "control-decider"],
            ROOT / "tests/plugins/control-timeout/timeout.config.toml",
        )
        assert_load_fails(
            actraild,
            DUPLICATE_EVENT_FAMILIES_MANIFEST,
            DUPLICATE_EVENT_FAMILIES_INSTANCE,
            ["plugin_manifest", "subscriptions.event_families", "duplicates"],
        )
        assert_load_fails(
            actraild,
            DUPLICATE_CAPABILITIES_MANIFEST,
            DUPLICATE_CAPABILITIES_INSTANCE,
            ["plugin_manifest", "capabilities", "duplicates"],
            host_grants=["payload-read"],
        )
        assert_load_fails(
            actraild,
            BUILTIN_WASM_ABI_MANIFEST,
            BUILTIN_WASM_ABI_INSTANCE,
            ["plugin_manifest", "unused runtime sections", "runtime.wasm"],
        )
        assert_load_fails(
            actraild,
            MISSING_WASM_ARTIFACT_MANIFEST,
            MISSING_WASM_ARTIFACT_INSTANCE,
            ["plugin_manifest", "runtime.wasm"],
        )
        assert_load_fails(
            actraild,
            WHITESPACE_WASM_ARTIFACT_MANIFEST,
            WHITESPACE_WASM_ARTIFACT_INSTANCE,
            ["plugin_manifest", "runtime.wasm.artifact_path", "wasm"],
        )
        assert_load_fails(
            actraild,
            FILE_ACCESS_CURRENT_MATCH_GET_MANIFEST,
            UNGRANTED_FILE_ACCESS_CURRENT_MATCH_GET_INSTANCE,
            ["plugin_capability", "file-access.current-match-get"],
        )
        assert_load_fails(
            actraild,
            ZERO_FILE_ACCESS_CURRENT_MATCH_GET_MANIFEST,
            ZERO_FILE_ACCESS_CURRENT_MATCH_GET_INSTANCE,
            ["plugin_manifest", "hostcall_limits.file_policy.read_max_bytes", "greater than zero"],
        )
        assert_load_fails(
            actraild,
            CONTEXT_QUERY_MANIFEST,
            UNGRANTED_CONTEXT_QUERY_INSTANCE,
            ["plugin_capability", "context-query"],
        )
        assert_load_fails(
            actraild,
            ZERO_CONTEXT_QUERY_MANIFEST,
            ZERO_CONTEXT_QUERY_INSTANCE,
            ["plugin_manifest", "hostcall_limits.context.read_max_bytes", "greater than zero"],
        )
        assert_load_fails(
            actraild,
            NATIVE_OBSERVATION_MANIFEST,
            NATIVE_OBSERVATION_INSTANCE,
            ["plugin_factory", "native dynamic plugins are not enabled"],
        )
        assert_load_fails(
            actraild,
            NATIVE_CONTROL_MANIFEST,
            NATIVE_CONTROL_INSTANCE,
            ["plugin_factory", "native dynamic plugins are not enabled"],
        )
        assert_load_fails(
            actraild,
            NETWORK_EGRESS_MANIFEST,
            UNGRANTED_NETWORK_EGRESS_INSTANCE,
            ["plugin_capability", "network-egress"],
        )
        assert_load_fails(
            actraild,
            NETWORK_EGRESS_MANIFEST,
            UNSUPPORTED_NETWORK_EGRESS_GRANT_INSTANCE,
            ["plugin_capability", "unsupported plugin host grant", "network-egress"],
            host_grants=["network-egress:tcp=127.0.0.1:443"],
        )
        assert_load_fails(
            actraild,
            BUILTIN_HOST_CAPABILITY_MANIFEST,
            BUILTIN_HOST_CAPABILITY_INSTANCE,
            ["plugin_factory", "builtin plugin otel-jsonl", "host capabilities", "payload-read"],
            BUILTIN_HOST_CAPABILITY_CONFIG,
            ["payload-read"],
        )
        assert_load_fails(
            actraild,
            BUILTIN_ARTIFACT_PATH_MANIFEST,
            BUILTIN_ARTIFACT_PATH_INSTANCE,
            ["plugin_manifest", "unused runtime sections", "runtime.wasm"],
            BUILTIN_ARTIFACT_PATH_CONFIG,
        )
        print("plugin_load_validation_required_config=ok")
        print("plugin_load_validation_toml_parse=ok")
        print("plugin_load_validation_unsupported_format=ok")
        print("plugin_load_validation_empty_id=ok")
        print("plugin_load_validation_unsupported_api=ok")
        print("plugin_load_validation_missing_schema=ok")
        print("plugin_load_validation_empty_schema_ref=ok")
        print("plugin_load_validation_schema_mismatch=ok")
        print("plugin_load_validation_schema_match=ok")
        print("plugin_load_validation_empty_instance=ok")
        print("plugin_load_validation_empty_persistent_instance=ok")
        print("plugin_load_validation_empty_management_instance=ok")
        print("plugin_load_validation_whitespace_management_instance=ok")
        print("plugin_load_validation_tls_payload_read_source_grant=ok")
        print("plugin_load_validation_ungranted_capability=ok")
        print("plugin_load_validation_invalid_payload_read_source=ok")
        print("plugin_load_validation_mixed_payload_read_grants=ok")
        print("plugin_load_validation_duplicate_host_grant=ok")
        print("plugin_load_validation_unrequested_payload_read_source_grant=ok")
        print("plugin_load_validation_zero_control_concurrency=ok")
        print("plugin_load_validation_zero_payload_ref=ok")
        print("plugin_load_validation_zero_observation_queue=ok")
        print("plugin_load_validation_zero_plugin_config_read=ok")
        print("plugin_load_validation_zero_plugin_command_timeout=ok")
        print("plugin_load_validation_empty_event_families=ok")
        print("plugin_load_validation_control_event_families=ok")
        print("plugin_load_validation_duplicate_event_families=ok")
        print("plugin_load_validation_duplicate_capabilities=ok")
        print("plugin_load_validation_builtin_wasm_abi=ok")
        print("plugin_load_validation_missing_wasm_artifact=ok")
        print("plugin_load_validation_whitespace_wasm_artifact=ok")
        print("plugin_load_validation_ungranted_current_match_get=ok")
        print("plugin_load_validation_zero_current_match_get=ok")
        print("plugin_load_validation_ungranted_context_query=ok")
        print("plugin_load_validation_zero_context_query=ok")
        print("plugin_load_validation_native_observation_disabled=ok")
        print("plugin_load_validation_native_control_disabled=ok")
        print("plugin_load_validation_future_network_egress_disabled=ok")
        print("plugin_load_validation_builtin_host_capability_disabled=ok")
        print("plugin_load_validation_builtin_artifact_path_disabled=ok")
        return 0
    finally:
        daemon.send_signal(signal.SIGINT)
        try:
            stdout, _ = daemon.communicate(timeout=10)
        except subprocess.TimeoutExpired:
            daemon.kill()
            stdout, _ = daemon.communicate(timeout=10)
        if daemon.returncode not in (0, -signal.SIGINT):
            raise RuntimeError(f"daemon exited with {daemon.returncode}\n{stdout[-4000:]}")


if __name__ == "__main__":
    raise SystemExit(main())
