#!/usr/bin/env python3
"""Run all v2 regression cases through the common test runner."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))

from tests.v2.common.core import TestOutput
from tests.v2.common.runner import add_common_arguments, run_selected
from tests.v2.regression.local_profile import (  # noqa: E402
    DEFAULT_PROFILE,
    load_local_test_profile,
)
from tests.v2.regression.probe_claude_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CLAUDE,
)
from tests.v2.regression.probe_claude_mcp.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CLAUDE_MCP,
)
from tests.v2.regression.probe_codex_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CODEX,
)
from tests.v2.regression.probe_codex_mcp.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CODEX_MCP,
)
from tests.v2.regression.probe_pi_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as PI,
)
from tests.v2.regression.probe_qodercli_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as QODERCLI,
)
from tests.v2.regression.probe_xiaoo_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as XIAOO,
)
from tests.v2.regression.command_policy_xiaoo.run_e2e import (  # noqa: E402
    TEST_DEFINITION as COMMAND_POLICY_XIAOO,
)
from tests.v2.regression.network_policy_xiaoo.run_e2e import (  # noqa: E402
    TEST_DEFINITION as NETWORK_POLICY_XIAOO,
)
from tests.v2.regression.activity_anomaly.run_e2e import (  # noqa: E402
    TEST_DEFINITION as ACTIVITY_ANOMALY,
)
from tests.v2.regression.alert_forwarding.run_e2e import (  # noqa: E402
    TEST_DEFINITION as ALERT_FORWARDING,
)
from tests.v2.regression.container_agent_xiaoo.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CONTAINER_AGENT_XIAOO,
)
from tests.v2.regression.container_auto.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CONTAINER_AUTO,
)
from tests.v2.regression.execution_isolation_cloud_hypervisor.run_e2e import (  # noqa: E402
    TEST_DEFINITION as EXECUTION_ISOLATION_CLOUD_HYPERVISOR,
)
from tests.v2.regression.sandbox_resource_alert_host.run_e2e import (  # noqa: E402
    TEST_DEFINITION as SANDBOX_RESOURCE_ALERT_HOST,
)
from tests.v2.regression.otel_jsonl_action_filter.run_e2e import (  # noqa: E402
    TEST_DEFINITION as OTEL_JSONL_ACTION_FILTER,
)
from tests.v2.regression.otel_http.run_e2e import (  # noqa: E402
    TEST_DEFINITION as OTEL_HTTP,
)
from tests.v2.regression.project_subagent_trajectory.run_e2e import (  # noqa: E402
    TEST_DEFINITION as PROJECT_SUBAGENT_TRAJECTORY,
)
from tests.v2.regression.semantic_action_boundaries.run_e2e import (  # noqa: E402
    TEST_DEFINITION as SEMANTIC_ACTION_BOUNDARIES,
)
from tests.v2.regression.virtual_container.run_e2e import (  # noqa: E402
    TEST_DEFINITION as VIRTUAL_CONTAINER,
)
from tests.v2.regression.virtual_container_xiaoo_concurrency.run_e2e import (  # noqa: E402
    TEST_DEFINITION as VIRTUAL_CONTAINER_XIAOO_CONCURRENCY,
)
from tests.v2.regression.tool_consecutive_failure_alert.run_e2e import (  # noqa: E402
    TEST_DEFINITION as TOOL_CONSECUTIVE_FAILURE_ALERT,
)
from tests.v2.regression.tool_frequent_failure_alert.run_e2e import (  # noqa: E402
    TEST_DEFINITION as TOOL_FREQUENT_FAILURE_ALERT,
)

DEFAULT_TESTS = [
    CLAUDE,
    CLAUDE_MCP,
    CODEX,
    CODEX_MCP,
    PI,
    QODERCLI,
    XIAOO,
    COMMAND_POLICY_XIAOO,
    NETWORK_POLICY_XIAOO,
    VIRTUAL_CONTAINER,
    VIRTUAL_CONTAINER_XIAOO_CONCURRENCY,
    SANDBOX_RESOURCE_ALERT_HOST,
    CONTAINER_AUTO,
    CONTAINER_AGENT_XIAOO,
    SEMANTIC_ACTION_BOUNDARIES,
    OTEL_JSONL_ACTION_FILTER,
    OTEL_HTTP,
    PROJECT_SUBAGENT_TRAJECTORY,
    ACTIVITY_ANOMALY,
    ALERT_FORWARDING,
    TOOL_CONSECUTIVE_FAILURE_ALERT,
    TOOL_FREQUENT_FAILURE_ALERT,
]

OPTIONAL_TESTS = [
    EXECUTION_ISOLATION_CLOUD_HYPERVISOR,
]

TESTS = [*DEFAULT_TESTS, *OPTIONAL_TESTS]


def main(argv: list[str] | None = None) -> int:
    effective_argv = list(sys.argv[1:] if argv is None else argv)
    bootstrap_parser = argparse.ArgumentParser(add_help=False)
    profile_group = bootstrap_parser.add_mutually_exclusive_group()
    profile_group.add_argument("--profile", type=Path)
    profile_group.add_argument("--no-profile", action="store_true")
    bootstrap, _ = bootstrap_parser.parse_known_args(effective_argv)
    try:
        loaded_profile = (
            None
            if bootstrap.no_profile
            else load_local_test_profile(REPO, bootstrap.profile)
        )
    except ValueError as error:
        bootstrap_parser.error(str(error))

    parser = argparse.ArgumentParser(description="Run AcTrail v2 regression tests")
    add_common_arguments(parser)
    profile_group = parser.add_mutually_exclusive_group()
    profile_group.add_argument(
        "--profile",
        type=Path,
        help=(
            "machine-local test profile "
            f"(default when present: {DEFAULT_PROFILE})"
        ),
    )
    profile_group.add_argument(
        "--no-profile",
        action="store_true",
        help="do not load the machine-local test profile",
    )
    parser.add_argument(
        "--case",
        action="append",
        choices=[test.name for test in TESTS],
        dest="cases",
        help="case to run; repeatable (default: default regression set)",
    )
    parser.add_argument(
        "--list",
        action="store_true",
        dest="list_cases",
        help="list available cases without running them",
    )
    parser.add_argument(
        "--fail-fast",
        action="store_true",
        help="stop after the first failed case",
    )
    arguments = parser.parse_args(effective_argv)
    requested = set(arguments.cases or ())
    if requested:
        selected = [test for test in TESTS if test.name in requested]
    elif arguments.list_cases:
        selected = TESTS
    else:
        selected = DEFAULT_TESTS
    if loaded_profile is not None:
        TestOutput(color_mode=arguments.color).line(
            f"test_profile={loaded_profile}"
        )
    if arguments.list_cases:
        output = TestOutput(color_mode=arguments.color)
        for test in selected:
            output.line(f"{test.name}: {test.description}")
        return 0
    return run_selected(
        selected,
        REPO,
        arguments.bin_dir,
        arguments.color,
        arguments.log_dir,
        arguments.work_root,
        show_details=False,
        cleanup_cases=arguments.cleanup,
        fail_fast=arguments.fail_fast,
        lock_path=arguments.lock_path,
        lock_timeout_seconds=arguments.lock_timeout_seconds,
        lock_poll_seconds=arguments.lock_poll_seconds,
    )


if __name__ == "__main__":
    raise SystemExit(main())
