"""Regression coverage for documented example transfer checks."""

from __future__ import annotations

import sys
from pathlib import Path

from model import FAIL, PASS, SKIP, CaseResult
from workload_config import read_config

CASE_DIR = Path(__file__).resolve().parent
if str(CASE_DIR) not in sys.path:
    sys.path.insert(0, str(CASE_DIR))

from case_01_quick_start.step import run_quick_start
from case_02_llm_http_payload_capture.step import (
    run_external_http1,
    run_external_http2,
    run_http2_local,
)
from case_03_extended_observation_e2e.step import run_extended_observation
from case_07_xiaoo_claude_agent_invocation.step import run_agent_invocation
from case_08_full_monitor_validation.step import run_full_monitor_python_launch


CASE_ID = "docs-examples"
TITLE = "Documented examples transfer checks"
SUITES = {"quick", "full"}
WORKLOAD_CONFIG = CASE_DIR / "workload.conf"


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    workload = read_config(WORKLOAD_CONFIG)
    if not env.is_root():
        result.status = SKIP
        result.add_check(
            "root privileges",
            SKIP,
            "docs examples require eBPF/seccomp/fanotify capable root runs",
            "the documented transfer checks are compiled-binary E2E workflows",
        )
        return result
    if missing := missing_binaries(env, ("actraild", "actrailctl", "actrailviewer", "ebpf_probe")):
        result.status = SKIP
        result.add_check(
            "release binaries",
            SKIP,
            "missing " + ", ".join(missing),
            "run cargo build --release before docs example regression",
        )
        return result
    for step in (
        run_quick_start,
        run_http2_local,
        run_external_http1,
        run_external_http2,
        run_extended_observation,
        run_agent_invocation,
        run_full_monitor_python_launch,
    ):
        status = step(env, result, workload)
        if status == FAIL:
            result.status = FAIL
            return result
    if any(check.status == SKIP for check in result.checks):
        result.status = SKIP
    return result


def missing_binaries(env, names: tuple[str, ...]) -> list[str]:
    return [name for name in names if env.release_binary(name) is None]
