#!/usr/bin/env python3
"""Resolve the concrete Claude Code TLS runtime for payload E2E tests."""

from __future__ import annotations

import os
import shutil
import sys
from dataclasses import dataclass
from pathlib import Path


REPO_ROOT = Path(__file__).resolve().parents[3]
TLS_RUNTIME_DIR = REPO_ROOT / "tests/agent-trace/runtime_tls"
if str(TLS_RUNTIME_DIR) not in sys.path:
    sys.path.insert(0, str(TLS_RUNTIME_DIR))

from fast_plan import resolve_fast_probe_plan  # noqa: E402


@dataclass(frozen=True)
class ClaudeTlsRuntime:
    provider: str
    detail: str


def resolve_claude_tls_runtime(settings: dict[str, str], finder: Path) -> ClaudeTlsRuntime:
    explicit_binary = os.environ.get("CLAUDE_TLS_BINARY")
    if explicit_binary:
        target: Path | str = require_existing_file(Path(explicit_binary))
    else:
        if shutil.which("claude") is None:
            raise RuntimeError("claude CLI is not on PATH")
        target = "claude"
    plan = resolve_fast_probe_plan(
        target,
        finder,
        required(settings, "tls_probe_provider"),
        required(settings, "tls_probe_source"),
        required(settings, "tls_probe_match_limit"),
    )
    return ClaudeTlsRuntime(provider=plan.provider, detail=plan.detail)

def require_existing_file(path: Path) -> Path:
    resolved = path.resolve()
    if not resolved.is_file():
        raise RuntimeError(f"not a file: {path}")
    return resolved


def required(values: dict[str, str], key: str) -> str:
    value = values.get(key)
    if not value:
        raise RuntimeError(f"missing Claude workload config key {key}")
    return value
