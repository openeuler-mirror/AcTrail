"""Platform and runtime initialization checks."""

from __future__ import annotations

import os
from pathlib import Path

from evidence import expected_found_detail
from model import FAIL, PASS, WARN, CaseResult


CASE_ID = "initialize"
TITLE = "OS, environment, seccomp, eBPF, and binaries"
SUITES = {"quick", "full"}


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    result.add_check(
        "OS and environment",
        PASS,
        expected_found_detail(
            "kernel, architecture, and uid are recorded",
            platform_facts(env),
        ),
        "recorded kernel, architecture, and uid so platform-specific failures are attributable",
    )
    result.add_check(
        "proxy environment",
        PASS,
        expected_found_detail(
            "proxy variables are recorded for external provider cases",
            [env.proxy_summary()],
        ),
        "real agent/provider cases inherit these variables when external network access needs a proxy",
    )
    result.add_check(
        "effective uid",
        PASS if env.is_root() else FAIL,
        expected_found_detail("uid=0", [f"uid={os.geteuid()}"]),
        "root is required for the eBPF, seccomp notify, and fanotify E2E cases",
    )
    add_release_binary_checks(result, env)
    result.add_check(
        "seccomp user notification",
        seccomp_status(),
        expected_found_detail("actions_avail contains user_notif", [f"actions_avail={seccomp_detail()}"]),
        "kernel must list user_notif before launch-time syscall pause/read paths can work",
    )
    btf_path = Path("/sys/kernel/btf/vmlinux")
    result.add_check(
        "eBPF BTF",
        PASS if btf_path.exists() else FAIL,
        expected_found_detail("BTF file exists", [f"path={btf_path}", f"exists={btf_path.exists()}"]),
        "BTF is required for loading the CO-RE eBPF collector",
    )
    tracefs_path = writable_tracefs_path()
    result.add_check(
        "tracefs",
        PASS if tracefs_path else FAIL,
        expected_found_detail("writable tracefs mount", [f"path={tracefs_path or 'missing'}"]),
        "tracefs availability is needed for live probe/platform checks",
    )
    preflight = env.run(
        [
            env.python,
            str(env.repo_root / "docs/preflight/platform_preflight.py"),
            "--color",
            "never",
        ]
    )
    result.command = preflight.command
    result.stdout_tail = env.output_tail(preflight.stdout)
    result.stderr_tail = env.output_tail(preflight.stderr)
    result.add_check(
        "platform preflight",
        PASS if preflight.returncode == 0 else FAIL,
        expected_found_detail(
            "docs/preflight/platform_preflight.py exits 0",
            [
                f"command={' '.join(preflight.command)}",
                f"exit={preflight.returncode}",
            ],
        ),
        f"preflight exited {preflight.returncode}; stdout tail in the report contains per-capability ticks",
    )
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    elif any(check.status == WARN for check in result.checks):
        result.status = WARN
    return result


def add_release_binary_checks(result: CaseResult, env) -> None:
    names = ("actraild", "actrailctl", "actrailviewer")
    for name in names:
        path = env.release_binary(name)
        result.add_check(
            f"{name} binary",
            PASS if path else FAIL,
            expected_found_detail(
                f"{name} release binary exists",
                [
                    f"path={path if path else env.bin_dir / name}",
                    f"exists={bool(path)}",
                ],
            ),
            "regression cases execute compiled release binaries, not source-level mocks",
        )


def seccomp_detail() -> str:
    path = Path("/proc/sys/kernel/seccomp/actions_avail")
    return path.read_text(encoding="utf-8", errors="ignore").strip() if path.exists() else "missing"


def seccomp_status() -> str:
    return PASS if "user_notif" in seccomp_detail().split() else FAIL


def writable_tracefs_path() -> Path | None:
    for path in (Path("/sys/kernel/tracing"), Path("/sys/kernel/debug/tracing")):
        if path.exists() and os.access(path, os.W_OK):
            return path
    return None


def platform_facts(env) -> list[str]:
    return [f"platform={env.platform_summary()}"]
