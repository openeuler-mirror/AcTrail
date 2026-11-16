"""Best-effort host environment summary printed at regression startup."""

from __future__ import annotations

import os
import platform
import socket
import subprocess
import tomllib
from pathlib import Path

from ..core import TestOutput
from ..testing_env import AgentBinaryDiscovery, default_claude_model

from .constants import DEFAULTS


class EnvironmentProbe:
    """Collect and print host characteristics once per regression run.

    Version and file queries use ACTRAIL_TEST_ENV_PROBE_TIMEOUT_SECONDS
    (default: 10). Unavailable metadata is left empty.
    """

    _OS_RELEASE_CANDIDATES = (
        Path("/etc/os-release"),
        Path("/usr/lib/os-release"),
    )
    _FIELD_ORDER = (
        "kernel_release",
        "os",
        "machine",
        "hostname",
        "python",
    )
    _AGENTS = DEFAULTS.environment_probe_agents

    def __init__(self, output: TestOutput, repo: Path) -> None:
        self._output = output
        self._discovery = AgentBinaryDiscovery(repo)
        self._timeout_seconds = float(
            os.environ.get("ACTRAIL_TEST_ENV_PROBE_TIMEOUT_SECONDS", str(DEFAULTS.environment_probe_timeout_seconds))
        )

    def print_summary(self) -> None:
        fields = self._collect()
        self._output.heading("▶ environment")
        for name in self._FIELD_ORDER:
            value = fields.get(name)
            if value is not None:
                self._output.line(f"{name}={value}")
        for name in self._AGENTS:
            binary = self._discovery.resolve(f"{name.upper()}_E2E_BINARY", name)
            if binary is not None:
                self._output.line(self._agent_summary(name, binary))

    def _collect(self) -> dict[str, str]:
        fields: dict[str, str] = {}
        uname = os.uname()
        fields["kernel_release"] = uname.release
        fields["machine"] = uname.machine
        fields["hostname"] = socket.gethostname()
        fields["python"] = platform.python_version()
        os_pretty_name = self._os_pretty_name()
        if os_pretty_name is not None:
            fields["os"] = os_pretty_name
        return fields

    def _agent_summary(self, name: str, binary: Path) -> str:
        native = binary.resolve()
        if name == "codex":
            environment = self._discovery.codex_environment(binary)
            native = self._discovery.codex_native_binary_for(binary, use_override=False) or native
            model = self._codex_model(binary)
        else:
            environment = self._discovery.environment(binary)
            model = (
                default_claude_model()
                if name == "claude"
                else self._xiaoo_model(environment)
            )
        version = self._command_output([str(binary), "--version"], environment)
        description = self._command_output(["file", "-L", "-b", str(native)], environment)
        description = ", ".join(
            field
            for field in description.split(", ")
            if not field.startswith(("version ", "interpreter ", "for ", "BuildID"))
        )
        summary = (
            f"[{name}] [model={model}][version={version}]"
            f"[path={native.resolve()}][bin={description}]"
        )
        if binary != native.resolve():
            summary += f"[entry={binary}]"
        if name == "codex" and (configured := os.environ.get("CODEX_E2E_NATIVE_BINARY")):
            probe_path = Path(configured).resolve()
            if probe_path != native.resolve():
                summary += f"[probe_path={probe_path}]"
        return summary

    def _command_output(self, command: list[str], environment: dict[str, str]) -> str:
        try:
            result = subprocess.run(
                command,
                env=environment,
                capture_output=True,
                text=True,
                timeout=self._timeout_seconds,
                check=False,
            )
        except (OSError, subprocess.TimeoutExpired):
            return ""
        return " ".join(result.stdout.split()) if result.returncode == 0 else ""

    def _codex_model(self, binary: Path) -> str:
        configured = os.environ.get("CODEX_E2E_MODEL")
        if configured:
            return configured
        model = self._discovery.default_codex_model_for_binary(binary) or "gpt-5.5"
        os.environ["CODEX_E2E_MODEL"] = model
        return model

    @staticmethod
    def _xiaoo_model(environment: dict[str, str]) -> str:
        path = Path(
            environment.get("XIAOO_CONFIG")
            or Path(environment["HOME"]) / ".config" / "xiaoo" / "config.toml"
        )
        try:
            with path.open("rb") as stream:
                return tomllib.load(stream)["llm"]["model"]
        except (OSError, tomllib.TOMLDecodeError, KeyError, TypeError):
            return ""

    def _os_pretty_name(self) -> str | None:
        for path in self._OS_RELEASE_CANDIDATES:
            pretty_name = self._read_pretty_name(path)
            if pretty_name is not None:
                return pretty_name
        return None

    @staticmethod
    def _read_pretty_name(path: Path) -> str | None:
        try:
            lines = path.read_text(encoding="utf-8").splitlines()
        except OSError:
            return None
        for line in lines:
            if not line.startswith("PRETTY_NAME="):
                continue
            value = line.split("=", 1)[1].strip()
            if len(value) >= 2 and value[0] == value[-1] == '"':
                value = value[1:-1]
            return value or None
        return None
