"""LangGraph OpenAI-compatible payload regression case."""

from __future__ import annotations

from pathlib import Path
from urllib.parse import urlparse

from e2e_steps.checks import StepFailure
from e2e_steps.loader import load_package
from evidence import expected_found_detail
from model import FAIL, PASS, SKIP, CaseResult

CASE_DIR = Path(__file__).resolve().parent
DIRECT_STEPS = load_package("regression_e2e_langgraph_direct_steps", CASE_DIR / "direct_steps")
run_direct_langgraph_case = DIRECT_STEPS.run_direct_langgraph_case


CASE_ID = "e2e-langgraph"
TITLE = "E2E with LangGraph Python OpenAI-compatible LLM request"
SUITES = {"quick", "agent", "payload", "full"}


def run(env) -> CaseResult:
    result = CaseResult(CASE_ID, TITLE, PASS, 0.0)
    explicit_python = env.has_env("LANGGRAPH_PYTHON")
    workload = read_config(env.repo_root / "tests/agent-trace/langgraph-openai/workload.conf")
    requires_tls = urlparse(workload.get("api_url", "")).scheme == "https"
    if not env.has_env("DEEPSEEK_API_KEY"):
        result.status = SKIP
        result.add_check(
            "DeepSeek API key",
            SKIP,
            expected_found_detail("DEEPSEEK_API_KEY is set", ["found=missing"]),
            "real LangGraph provider traffic cannot be generated without the key",
        )
        return result
    result.add_check(
        "DeepSeek API key",
        PASS,
        expected_found_detail("DEEPSEEK_API_KEY is set", ["found=present"]),
        "the workload can call the real OpenAI-compatible provider",
    )
    python, discovery_detail = select_langgraph_python(env, requires_tls)
    if python is None:
        result.status = FAIL if explicit_python else SKIP
        result.add_check(
            "LangGraph Python",
            result.status,
            expected_found_detail(
                "Python imports langgraph/requests and satisfies transport requirements",
                [discovery_detail],
            ),
            "the selected Python must import langgraph/requests and expose a supported transport path",
        )
        return result
    result.add_check(
        "LangGraph Python",
        PASS,
        expected_found_detail(
            "Python imports langgraph/requests and satisfies transport requirements",
            [discovery_detail],
        ),
        "the selected Python satisfies package and TLS/plain-HTTP requirements for the workload URL",
    )
    package_status = python_package_status(env, python, requires_tls)
    if any(check.status != PASS for check in package_status):
        result.status = FAIL if explicit_python else SKIP
        if explicit_python:
            for check in package_status:
                if check.status == SKIP:
                    check.status = FAIL
        for check in package_status:
            result.add_check(check.name, check.status, check.detail, check.evidence)
        return result
    for check in package_status:
        result.add_check(check.name, check.status, check.detail, check.evidence)
    if not env.release_binaries_ready():
        result.status = SKIP
        result.add_check(
            "release binaries",
            SKIP,
            expected_found_detail("compiled release binaries are present", ["missing one or more release binaries"]),
            "the E2E must use compiled AcTrail binaries",
        )
        return result
    try:
        run_direct_langgraph_case(env, result, workload, python)
    except StepFailure:
        return result
    if any(check.status == FAIL for check in result.checks):
        result.status = FAIL
    return result


def select_langgraph_python(env, requires_tls: bool) -> tuple[str | None, str]:
    explicit = env.has_env("LANGGRAPH_PYTHON")
    if explicit:
        raw = env.langgraph_python()
        resolved = env.resolve_executable_reference(raw)
        if resolved is None:
            return None, f"LANGGRAPH_PYTHON is not executable: {raw}"
        candidates = [str(resolved)]
    else:
        candidates = [str(path) for path in env.python_candidates()]
    failures = []
    for candidate in candidates:
        ready, detail = langgraph_python_ready(env, candidate, requires_tls)
        if ready:
            source = "LANGGRAPH_PYTHON" if explicit else "auto-detected"
            return candidate, f"{source}: {candidate}"
        failures.append(f"{candidate}: {detail}")
    if explicit:
        return None, "; ".join(failures)
    tls_detail = " and dynamic OpenSSL" if requires_tls else ""
    return None, f"no Python candidate has langgraph, requests{tls_detail}; scanned " + "; ".join(
        failures
    )


def langgraph_python_ready(env, python: str, requires_tls: bool) -> tuple[bool, str]:
    statuses = python_package_status(env, python, requires_tls)
    missing = [check.detail for check in statuses if check.status != PASS]
    return not missing, ", ".join(missing) if missing else "ready"


def python_package_status(env, python: str, requires_tls: bool) -> list:
    checks = []
    for package in ("langgraph", "requests"):
        result = env.run(
            [
                python,
                "-c",
                "import importlib.util, sys; sys.exit(0 if importlib.util.find_spec(sys.argv[1]) else 1)",
                package,
            ]
        )
        checks.append(
            make_check(
                f"Python package {package}",
                PASS if result.returncode == 0 else SKIP,
                expected_found_detail(
                    f"{python} can import {package}",
                    [
                        f"exit={result.returncode}",
                        f"package={package}",
                    ],
                ),
                f"importlib probe exited {result.returncode}",
            )
        )
    if not requires_tls:
        checks.append(
            make_check(
                "Python dynamic OpenSSL",
                PASS,
                expected_found_detail(
                    "dynamic OpenSSL is only required for HTTPS api_url",
                    ["api_url uses plain HTTP"],
                ),
                "the configured API URL uses HTTP, so socket plaintext capture can provide request evidence",
            )
        )
        return checks
    ssl_result = env.run(
        [
            python,
            "-c",
            "import _ssl, ssl, sys; "
            "print(getattr(_ssl, '__file__', '')); "
            "print(ssl.OPENSSL_VERSION); "
            "print(sys.executable); "
            "print(sys.base_prefix)",
        ]
    )
    ssl_lines = ssl_result.stdout.splitlines()
    ssl_path = ssl_lines[0] if ssl_lines else ""
    if ssl_result.returncode != 0 or not ssl_path:
        executable = ssl_lines[2] if len(ssl_lines) > 2 else python
        base_prefix = ssl_lines[3] if len(ssl_lines) > 3 else "unknown"
        checks.append(
            make_check(
                "Python dynamic OpenSSL",
                SKIP,
                expected_found_detail(
                    "_ssl extension path exists and links dynamic libssl",
                    [
                        f"executable={executable}",
                        f"base_prefix={base_prefix}",
                        "ssl_path=missing",
                    ],
                ),
                "the default HTTPS workload needs an _ssl extension linked to dynamic libssl",
            )
        )
        return checks
    ldd = env.run(["ldd", ssl_path])
    checks.append(
        make_check(
            "Python dynamic OpenSSL",
            PASS if ldd.returncode == 0 and "libssl" in ldd.stdout else SKIP,
            expected_found_detail(
                "_ssl extension links dynamic libssl",
                [
                    f"ssl_path={ssl_path}",
                    f"ldd_exit={ldd.returncode}",
                    f"links_libssl={'libssl' in ldd.stdout}",
                ],
            ),
            "dynamic libssl gives AcTrail a shared-library TLS attach target for HTTPS",
        )
    )
    return checks


def make_check(name: str, status: str, detail: str, evidence: str):
    from model import CheckResult

    return CheckResult(name, status, detail, evidence)


def read_config(path: Path) -> dict[str, str]:
    values = {}
    for raw in path.read_text(encoding="utf-8").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        key, separator, value = line.partition("=")
        if separator:
            values[key.strip()] = value.strip()
    return values
