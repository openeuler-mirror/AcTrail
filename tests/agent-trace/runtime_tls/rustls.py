#!/usr/bin/env python3
"""Resolve rustls plaintext probe points with tls-probe-point-finder."""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


RUSTLS_REQUIRED_HOOKS = (
    "rustls_buffer_plaintext",
    "rustls_take_received_plaintext",
)
ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")


@dataclass(frozen=True)
class RustlsProbePlan:
    build_id: str
    architecture: str
    symbols: dict[str, str]
    detail: str


def write_rustls_symbol_map(
    binary: Path,
    output: Path,
    settings: dict[str, str],
    finder: Path,
) -> str:
    plan = resolve_rustls_probe_plan(binary, settings, finder)
    lines = [
        "resolver = rustls-symbol-map",
        "library = rustls",
        f"arch = {plan.architecture}",
        f"build_id = {plan.build_id}",
    ]
    for symbol in RUSTLS_REQUIRED_HOOKS:
        lines.append(f"symbol = {symbol}|{plan.symbols[symbol]}")
    output.write_text("\n".join(lines) + "\n", encoding="utf-8")
    return plan.detail


def resolve_rustls_probe_plan(
    binary: Path,
    settings: dict[str, str],
    finder: Path,
) -> RustlsProbePlan:
    command = [
        str(finder),
        "fast",
        "--provider",
        required(settings, "rustls_probe_provider"),
        "--source",
        required(settings, "rustls_probe_source"),
        "--match-limit",
        required(settings, "rustls_probe_match_limit"),
        str(binary),
    ]
    result = subprocess.run(command, text=True, capture_output=True, check=False)
    output = strip_ansi(result.stdout)
    if result.returncode != 0:
        raise RuntimeError(
            "tls-probe-point-finder fast failed: "
            + (strip_ansi(result.stderr).strip() or output.strip())
        )
    plan = parse_finder_plan(output)
    missing = [symbol for symbol in RUSTLS_REQUIRED_HOOKS if symbol not in plan.symbols]
    if missing:
        raise RuntimeError(
            "rustls finder plan is missing required hook(s): " + ", ".join(missing)
        )
    return plan


def parse_finder_plan(output: str) -> RustlsProbePlan:
    if not re.search(r"(?m)^\s*provider = rustls\s*$", output):
        raise RuntimeError("finder plan did not select provider = rustls")
    build_id = required_match(output, r"(?m)^\s*build_id = ([0-9a-fA-F]+)\s*$", "build_id")
    architecture = required_match(
        output,
        r"(?m)^\s*architecture = ([A-Za-z0-9_.-]+)\s*$",
        "architecture",
    )
    symbols: dict[str, str] = {}
    current_symbol: str | None = None
    for raw_line in output.splitlines():
        line = raw_line.strip()
        symbol_match = re.match(r"- symbol = (\S+)$", line)
        if symbol_match:
            current_symbol = symbol_match.group(1)
            continue
        address_match = re.match(r"virtual_address = (0x[0-9a-fA-F]+)$", line)
        if address_match and current_symbol in RUSTLS_REQUIRED_HOOKS:
            symbols[current_symbol] = address_match.group(1)
    return RustlsProbePlan(
        build_id=build_id.lower(),
        architecture=architecture,
        symbols=symbols,
        detail=f"tls-probe-point-finder fast resolved rustls hooks for build_id={build_id.lower()}",
    )


def required_match(output: str, pattern: str, label: str) -> str:
    match = re.search(pattern, output)
    if match is None:
        raise RuntimeError(f"finder plan did not report {label}")
    return match.group(1)


def required(settings: dict[str, str], key: str) -> str:
    value = settings.get(key, "").strip()
    if not value:
        raise RuntimeError(f"missing workload key {key}")
    return value


def strip_ansi(value: str) -> str:
    return ANSI_ESCAPE.sub("", value)
