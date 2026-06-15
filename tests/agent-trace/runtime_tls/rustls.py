#!/usr/bin/env python3
"""Resolve rustls plaintext probe points with tls-probe-point-finder."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from fast_plan import resolve_fast_probe_plan

RUSTLS_REQUIRED_HOOKS = (
    "rustls_buffer_plaintext",
    "rustls_take_received_plaintext",
)


@dataclass(frozen=True)
class RustlsProbePlan:
    build_id: str
    architecture: str
    symbols: dict[str, str]
    detail: str


def resolve_rustls_probe_plan(
    binary: Path,
    settings: dict[str, str],
    finder: Path,
) -> RustlsProbePlan:
    plan = resolve_fast_probe_plan(
        binary,
        finder,
        required(settings, "rustls_probe_provider"),
        required(settings, "rustls_probe_source"),
        required(settings, "rustls_probe_match_limit"),
        RUSTLS_REQUIRED_HOOKS,
    )
    if plan.provider != "rustls":
        raise RuntimeError(f"finder plan did not select provider = rustls: {plan.provider}")
    return RustlsProbePlan(
        build_id=plan.build_id,
        architecture=plan.architecture,
        symbols=plan.symbols,
        detail=plan.detail,
    )


def required(settings: dict[str, str], key: str) -> str:
    value = settings.get(key, "").strip()
    if not value:
        raise RuntimeError(f"missing workload key {key}")
    return value
