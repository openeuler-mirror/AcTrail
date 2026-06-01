#!/usr/bin/env python3
"""Resolve the concrete Claude Code TLS runtime for payload E2E tests."""

from __future__ import annotations

import os
import platform
import shutil
import stat
import subprocess
import sys
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
TLS_RUNTIME_DIR = REPO_ROOT / "tests/agent-trace/runtime_tls"
if str(TLS_RUNTIME_DIR) not in sys.path:
    sys.path.insert(0, str(TLS_RUNTIME_DIR))

from boringssl import prepare_bun_static_boringssl_map  # noqa: E402


@dataclass(frozen=True)
class ClaudeTlsRuntime:
    binary: Path
    resolver: str
    library: str
    pattern_path: str
    detail: str


def resolve_claude_tls_runtime(settings: dict[str, str]) -> ClaudeTlsRuntime:
    explicit_binary = os.environ.get("CLAUDE_TLS_BINARY")
    if explicit_binary:
        binary = require_executable(Path(explicit_binary))
        return inspect_native_claude_binary(binary, settings, "CLAUDE_TLS_BINARY")

    claude = shutil.which("claude")
    if claude is None:
        raise RuntimeError("claude CLI is not on PATH")
    entry = Path(claude).resolve()
    node_runtime = resolve_node_launcher_runtime(entry)
    if node_runtime is not None:
        return node_runtime
    return inspect_native_claude_binary(entry, settings, "claude entrypoint")


def resolve_node_launcher_runtime(entry: Path) -> ClaudeTlsRuntime | None:
    try:
        first_line = entry.read_text(encoding="utf-8", errors="ignore").splitlines()[0]
    except (IndexError, OSError, UnicodeDecodeError):
        return None
    if "node" not in first_line:
        return None
    node = shutil.which("node")
    if node is None:
        raise RuntimeError(f"Claude entrypoint {entry} is Node-based, but node is not on PATH")
    binary = Path(node).resolve()
    missing = missing_exported_symbols(binary, ("SSL_read", "SSL_write", "SSL_read_ex", "SSL_write_ex"))
    if missing:
        raise RuntimeError(f"node runtime {binary} is missing {', '.join(missing)}")
    return ClaudeTlsRuntime(
        binary=binary,
        resolver="openssl-symbols",
        library="openssl",
        pattern_path="disabled",
        detail=f"Node/OpenSSL runtime {binary} exports SSL_read, SSL_write, SSL_read_ex, and SSL_write_ex",
    )


def inspect_native_claude_binary(
    binary: Path,
    settings: dict[str, str],
    source: str,
) -> ClaudeTlsRuntime:
    if not is_elf(binary):
        raise RuntimeError(f"{source} {binary} is neither a Node launcher nor an ELF executable")
    missing = missing_exported_symbols(binary, ("SSL_read", "SSL_write", "SSL_read_ex", "SSL_write_ex"))
    if not missing:
        return ClaudeTlsRuntime(
            binary=binary,
            resolver="openssl-symbols",
            library="openssl",
            pattern_path="disabled",
            detail=f"native OpenSSL executable {binary} exports SSL_read, SSL_write, SSL_read_ex, and SSL_write_ex",
        )
    if platform.machine() in {"aarch64", "x86_64"}:
        return ClaudeTlsRuntime(
            binary=binary,
            resolver="boringssl-static",
            library="boringssl",
            pattern_path="disabled",
            detail=f"native/static BoringSSL executable {binary}; using built-in related-entry detector",
        )

    configured_map = resolve_path(required(settings, "symbol_map_path"), REPO_ROOT)
    try:
        symbol_map, detail = prepare_bun_static_boringssl_map(binary, configured_map, settings)
    except Exception as error:
        symbol_detail = ", ".join(missing)
        raise RuntimeError(
            f"{source} {binary} does not expose OpenSSL symbols ({symbol_detail}) "
            f"and static BoringSSL discovery failed: {error}. "
            "Run python3 docs/preflight/claude_native_profile.py on this host to collect "
            "the build-id-bound native TLS profile diagnostics without exporting the binary."
        ) from error
    return ClaudeTlsRuntime(
        binary=binary,
        resolver="bun-static-boringssl",
        library="boringssl",
        pattern_path=str(symbol_map),
        detail=f"native/static BoringSSL executable {binary}; {detail}",
    )


def missing_exported_symbols(binary: Path, symbols: tuple[str, ...]) -> list[str]:
    output = run_checked(["readelf", "-Ws", str(binary)])
    exported = set()
    for line in output.splitlines():
        parts = line.split()
        if len(parts) < 8 or not parts[0].endswith(":"):
            continue
        if parts[3] != "FUNC" or parts[6] == "UND":
            continue
        exported.add(parts[7].split("@", 1)[0])
    return [symbol for symbol in symbols if symbol not in exported]


def require_executable(path: Path) -> Path:
    resolved = path.resolve()
    try:
        mode = resolved.stat().st_mode
    except OSError as error:
        raise RuntimeError(f"not an executable: {path}") from error
    if not stat.S_ISREG(mode) or not os.access(resolved, os.X_OK):
        raise RuntimeError(f"not an executable: {path}")
    return resolved


def is_elf(path: Path) -> bool:
    try:
        return path.read_bytes()[:4] == b"\x7fELF"
    except OSError:
        return False


def resolve_path(raw: str, repo: Path) -> Path:
    path = Path(raw)
    return path if path.is_absolute() else repo / path


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing Claude workload config key {key}")
    return value


def run_checked(command: list[str]) -> str:
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(
            f"command failed: {' '.join(command)}\nstdout={result.stdout}\nstderr={result.stderr}"
        )
    return result.stdout
