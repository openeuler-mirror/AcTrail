#!/usr/bin/env python3
"""Resolve rustls plaintext write symbols for executable TLS tests."""

from __future__ import annotations

import os
import platform
import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True)
class RustlsSymbols:
    build_id: str
    write: str
    write_vectored: str
    source: Path
    detail: str


def write_rustls_symbol_map(binary: Path, output: Path, settings: dict[str, str]) -> str:
    symbols = resolve_rustls_plaintext_symbols(binary, settings)
    lines = [
        "resolver = rustls-symbol-map",
        "library = rustls",
        f"arch = {platform.machine()}",
        f"build_id = {symbols.build_id}",
        f"symbol = rustls_plaintext_write|0x{symbols.write}",
        f"symbol = rustls_plaintext_write_vectored|0x{symbols.write_vectored}",
    ]
    output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return symbols.detail


def resolve_rustls_plaintext_symbols(binary: Path, settings: dict[str, str]) -> RustlsSymbols:
    build_id = binary_build_id(binary)
    attempts = []
    for source, label in rustls_symbol_sources(binary, settings, build_id):
        symbols, detail = rustls_symbols_from_file(source)
        if len(symbols) == 2:
            return RustlsSymbols(
                build_id=build_id,
                write=symbols["rustls_plaintext_write"],
                write_vectored=symbols["rustls_plaintext_write_vectored"],
                source=source,
                detail=f"{label} exposes rustls PlaintextSink symbols at {source}",
            )
        attempts.append(f"{label} {source}: {detail}")
    raise RuntimeError(
        "could not resolve rustls PlaintextSink write symbols; checked "
        + "; ".join(attempts)
    )


def rustls_symbol_sources(
    binary: Path,
    settings: dict[str, str],
    build_id: str,
) -> list[tuple[Path, str]]:
    sources = [(binary, "main binary symbol table")]
    explicit = os.environ.get("XIAOO_DEBUG_FILE") or configured_path(settings, "rustls_debug_file_path")
    if explicit:
        sources.append((Path(explicit).resolve(), "configured debuginfo"))
    build_id_debug = build_id_debug_file(settings, build_id)
    if build_id_debug is not None:
        sources.append((build_id_debug, "build-id debuginfo"))
    return unique_existing_sources(sources)


def configured_path(settings: dict[str, str], key: str) -> str | None:
    value = settings.get(key, "").strip()
    if not value or value in {"auto", "disabled"}:
        return None
    return value


def build_id_debug_file(settings: dict[str, str], build_id: str) -> Path | None:
    directory = settings.get("rustls_debug_build_id_directory", "").strip()
    if not directory or directory == "disabled" or len(build_id) < 3:
        return None
    path = Path(directory) / build_id[:2] / f"{build_id[2:]}.debug"
    return path.resolve()


def unique_existing_sources(sources: list[tuple[Path, str]]) -> list[tuple[Path, str]]:
    seen: set[Path] = set()
    output: list[tuple[Path, str]] = []
    for path, label in sources:
        if path in seen:
            continue
        seen.add(path)
        if path.exists():
            output.append((path, label))
        else:
            output.append((path, f"{label} missing"))
    return output


def rustls_symbols_from_file(path: Path) -> tuple[dict[str, str], str]:
    result = subprocess.run(["nm", "-C", str(path)], text=True, capture_output=True, check=False)
    if result.returncode != 0:
        return {}, result.stderr.strip() or f"nm failed with exit={result.returncode}"
    symbols: dict[str, str] = {}
    for line in result.stdout.splitlines():
        if "PlaintextSink>::write_vectored" in line and line.endswith("PlaintextSink>::write_vectored"):
            symbols["rustls_plaintext_write_vectored"] = line.split()[0]
        elif "PlaintextSink>::write" in line and line.endswith("PlaintextSink>::write"):
            symbols["rustls_plaintext_write"] = line.split()[0]
    missing = [
        name
        for name in ("rustls_plaintext_write", "rustls_plaintext_write_vectored")
        if name not in symbols
    ]
    if missing:
        return symbols, "missing " + ", ".join(missing)
    return symbols, "found rustls PlaintextSink write symbols"


def binary_build_id(binary: Path) -> str:
    result = subprocess.run(["readelf", "-n", str(binary)], text=True, capture_output=True, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"readelf -n failed for {binary}: {result.stderr}")
    match = re.search(r"Build ID:\s*([0-9a-fA-F]+)", result.stdout)
    if not match:
        raise RuntimeError(f"{binary} has no GNU build-id")
    return match.group(1).lower()
