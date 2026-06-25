"""Config and command helpers for agent trace E2E cases."""

from __future__ import annotations

import os
import subprocess
import sys
from pathlib import Path


DEFAULT_OPERATOR_CONFIG_PATH = Path("/etc/actrail/actraild.conf")


def operator_config_path(config: Path | None) -> Path:
    return config if config is not None else DEFAULT_OPERATOR_CONFIG_PATH


def actrail_command(binary: Path, config: Path | None, *args: str) -> list[str]:
    command = [str(binary)]
    if config is not None:
        command.extend(["--config", str(config)])
    command.extend(args)
    return command


def repo_root() -> Path:
    return Path(__file__).resolve().parents[3]


def read_config(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    section = ""
    array_depth = 0
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if array_depth:
            array_depth += line.count("[") - line.count("]")
            if array_depth < 0:
                raise RuntimeError(f"invalid config array in {path}: {raw}")
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]")
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid config line in {path}: {raw}")
        array_depth = value.count("[") - value.count("]")
        if array_depth < 0:
            raise RuntimeError(f"invalid config array in {path}: {raw}")
        remapped = operator_config_key(section, key.strip())
        if remapped is not None:
            values[remapped] = unquote(value.strip())
    if array_depth:
        raise RuntimeError(f"unterminated config array in {path}")
    return values


def operator_config_key(section: str, key: str) -> str | None:
    if section == "control" and key in {"socket_path", "pid_file", "log_path"}:
        return key
    if section == "web" and key == "listen_addr":
        return "web_listen_addr"
    if section == "web" and key == "request_read_timeout_ms":
        return "web_request_read_timeout_ms"
    if section == "storage.sqlite" and key == "path":
        return "storage_sqlite_path"
    if section == "export.snapshot" and key == "directory":
        return "export_directory"
    if section == "export.runtime" and key == "enabled":
        return "export_enabled"
    if section == "export.runtime.routes.otel_jsonl" and key == "path":
        return "export_otel_jsonl_path"
    if section == "payload.tls" and key == "sync_event_socket_path":
        return "payload_tls_sync_event_socket_path"
    if not section:
        return key
    return None


def unquote(value: str) -> str:
    if len(value) >= 2 and value.startswith('"') and value.endswith('"'):
        return value[1:-1]
    return value


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing config key {key}")
    return value


def require_root() -> None:
    if os.geteuid() != 0:
        raise RuntimeError("agent trace E2E requires root or equivalent eBPF/seccomp privileges")


def require_binary(bin_dir: Path, name: str) -> Path:
    path = bin_dir / name
    if not path.exists():
        raise RuntimeError(f"missing binary {path}; run cargo build --release")
    return path


def run_checked(
    command: list[str],
    *,
    echo: bool = True,
    timeout: float | None = None,
    cwd: Path | None = None,
) -> str:
    result = subprocess.run(
        command,
        text=True,
        capture_output=True,
        check=False,
        timeout=timeout,
        cwd=cwd,
    )
    if echo and result.stdout:
        print(result.stdout, end="", flush=True)
    if echo and result.stderr:
        print(result.stderr, end="", file=sys.stderr, flush=True)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout


def render_config(template: Path, output: Path, replacements: dict[str, str]) -> None:
    raw = template.read_text(encoding="utf-8")
    for placeholder, value in replacements.items():
        if placeholder not in raw:
            raise RuntimeError(f"{template} does not contain {placeholder}")
        raw = raw.replace(placeholder, value)
    output.write_text(raw, encoding="utf-8")


def clean_configured_paths(actrailctl: Path, config: Path | None) -> None:
    run_checked(actrail_command(actrailctl, config, "clean"))
