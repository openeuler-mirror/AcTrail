#!/usr/bin/env python3
"""Resolve TLS sync auto plans with tls-probe-point-finder fast."""

from __future__ import annotations

import re
import subprocess
from dataclasses import dataclass
from pathlib import Path


ANSI_ESCAPE = re.compile(r"\x1b\[[0-9;]*m")


@dataclass(frozen=True)
class FastProbePlan:
    provider: str
    build_id: str
    architecture: str
    symbols: dict[str, str]
    detail: str


def resolve_fast_probe_plan(
    binary: Path | str,
    finder: Path,
    provider: str,
    source: str,
    match_limit: str,
    required_symbols: tuple[str, ...] = (),
) -> FastProbePlan:
    command = [
        str(finder),
        "fast",
        "--provider",
        provider,
        "--source",
        source,
        "--match-limit",
        match_limit,
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
    missing = [symbol for symbol in required_symbols if symbol not in plan.symbols]
    if missing:
        raise RuntimeError("finder plan is missing required hook(s): " + ", ".join(missing))
    return plan


def parse_finder_plan(output: str) -> FastProbePlan:
    provider = required_match(output, r"(?m)^\s*provider = ([A-Za-z0-9_.-]+)\s*$", "provider")
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
        if address_match and current_symbol is not None:
            symbols[current_symbol] = address_match.group(1)
    return FastProbePlan(
        provider=provider,
        build_id=build_id.lower(),
        architecture=architecture,
        symbols=symbols,
        detail=(
            f"tls-probe-point-finder fast resolved provider={provider} "
            f"build_id={build_id.lower()}"
        ),
    )


def required_match(output: str, pattern: str, label: str) -> str:
    match = re.search(pattern, output)
    if match is None:
        raise RuntimeError(f"finder plan did not report {label}")
    return match.group(1)


def strip_ansi(value: str) -> str:
    return ANSI_ESCAPE.sub("", value)
