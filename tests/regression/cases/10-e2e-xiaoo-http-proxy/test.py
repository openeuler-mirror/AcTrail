"""xiaoO plain HTTP provider proxy regression case."""

from __future__ import annotations

import os
from pathlib import Path

from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult
from workload_config import read_config, required


CASE_ID = "e2e-xiaoo-http-proxy"
TITLE = "E2E with xiaoO through a plain HTTP provider proxy"
SUITES = {"agent", "payload", "full"}
WORKLOAD_CONFIG = "tests/agent-trace/xiaoo-http-proxy/workload.conf"
RUNNER = "tests/agent-trace/xiaoo-http-proxy/run_e2e.py"


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    workload = read_config(env.repo_root / WORKLOAD_CONFIG)
    if not env.is_root():
        return skip_case(
            result,
            "root privileges",
            "uid=0",
            ["root privileges missing"],
            "socket payload probes cannot be loaded without root/eBPF privileges",
        )
    result.add_check(
        "root privileges",
        PASS,
        expected_found_detail("uid=0", ["uid=0"]),
        "socket payload probes can be loaded",
    )
    if not release_binaries_ready(env):
        return skip_case(
            result,
            "release binaries",
            "compiled release binaries are present",
            ["missing one or more release binaries"],
            "the E2E must use compiled AcTrail binaries",
        )
    result.add_check(
        "release binaries",
        PASS,
        expected_found_detail("compiled release binaries are present", ["all required binaries present"]),
        "the E2E can run daemon, ctl, viewer, and web binaries",
    )
    selected = select_xiaoo_binary(env, os.environ.get("XIAOO_BINARY"))
    if selected is None:
        configured = os.environ.get("XIAOO_BINARY")
        status = FAIL if configured else SKIP
        result.status = status
        result.add_check(
            "xiaoO existence",
            status,
            expected_found_detail(
                "xiaoO executable is available",
                [f"configured={configured or 'unset'}", "found=missing"],
            ),
            "real xiaoO provider traffic cannot be generated without the CLI/binary",
        )
        return result
    result.add_check(
        "xiaoO existence",
        PASS,
        expected_found_detail("xiaoO executable is available", [f"path={selected}"]),
        "the case can launch a real xiaoO process",
    )
    upstream_key_env = required(workload, "upstream_api_key_env")
    if not env.has_env(upstream_key_env):
        return skip_case(
            result,
            "upstream API key",
            f"{upstream_key_env} is set",
            [f"{upstream_key_env}=missing"],
            "the local proxy needs a real upstream provider credential",
        )
    result.add_check(
        "upstream API key",
        PASS,
        expected_found_detail(f"{upstream_key_env} is set", [f"{upstream_key_env}=present"]),
        "the local proxy can forward xiaoO requests to the real HTTPS provider",
    )
    run_direct_case(env, result, workload, selected)
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result


def run_direct_case(
    env,
    result: CaseResult,
    workload: dict[str, str],
    selected: Path,
) -> None:
    old_binary = os.environ.get("XIAOO_BINARY")
    os.environ["XIAOO_BINARY"] = str(selected)
    try:
        command = [
            env.python,
            str(env.repo_root / RUNNER),
            "--bin-dir",
            str(env.bin_dir),
        ]
        result.begin_check("xiaoO HTTP proxy capture", "running direct E2E")
        completed = env.run(
            command,
            timeout=float(required(workload, "regression_timeout_seconds")),
        )
    finally:
        restore_env("XIAOO_BINARY", old_binary)
    result.command = completed.command
    result.stdout_tail = env.output_tail(completed.stdout)
    result.stderr_tail = env.output_tail(completed.stderr)
    if (
        completed.returncode == 0
        and "xiaoO HTTP proxy agent trace e2e complete" in completed.output
    ):
        result.add_check(
            "xiaoO HTTP proxy capture",
            PASS,
            expected_found_detail(
                "direct E2E exits 0 and reports completion",
                ["exit=0", "completion_marker=present"],
            ),
            "xiaoO used the generated plain HTTP provider config and AcTrail captured the exchange",
        )
        return
    result.status = FAIL
    result.add_check(
        "xiaoO HTTP proxy capture",
        FAIL,
        expected_found_detail(
            "direct E2E exits 0 and reports completion",
            [
                f"exit={completed.returncode}",
                f"completion_marker={'present' if 'xiaoO HTTP proxy agent trace e2e complete' in completed.output else 'missing'}",
            ],
        ),
        "direct xiaoO HTTP proxy E2E failed",
    )


def skip_case(
    result: CaseResult,
    name: str,
    expected: str,
    found: list[str],
    evidence: str,
) -> CaseResult:
    result.status = SKIP
    result.add_check(name, SKIP, expected_found_detail(expected, found), evidence)
    return result


def release_binaries_ready(env) -> bool:
    return all(
        env.release_binary(name)
        for name in ("actraild", "actrailctl", "actrailviewer", "actrailweb")
    )


def select_xiaoo_binary(env, configured: str | None) -> Path | None:
    if configured:
        return env.resolve_executable_reference(configured)
    candidates = env.executable_candidates("xiaoo")
    return candidates[0] if candidates else None


def restore_env(name: str, value: str | None) -> None:
    if value is None:
        os.environ.pop(name, None)
    else:
        os.environ[name] = value
