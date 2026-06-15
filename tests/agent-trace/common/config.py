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
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            section = line.strip("[]")
            continue
        if section.startswith("export"):
            continue
        key, separator, value = line.partition("=")
        if not separator:
            raise RuntimeError(f"invalid config line in {path}: {raw}")
        values[key.strip()] = value.strip()
    return values


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
