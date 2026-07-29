#!/usr/bin/env python3
"""Run all v2 regression cases through the common test runner."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))

from tests.v2.common.output import TestOutput  # noqa: E402
from tests.v2.common.runner import (  # noqa: E402
    add_common_arguments,
    run_selected,
)
from tests.v2.regression.probe_claude_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CLAUDE,
)
from tests.v2.regression.probe_codex_llm.run_e2e import (  # noqa: E402
    TEST_DEFINITION as CODEX,
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
from tests.v2.regression.otel_jsonl_action_filter.run_e2e import (  # noqa: E402
    TEST_DEFINITION as OTEL_JSONL_ACTION_FILTER,
)
from tests.v2.regression.semantic_action_boundaries.run_e2e import (  # noqa: E402
    TEST_DEFINITION as SEMANTIC_ACTION_BOUNDARIES,
)

TESTS = [
    CLAUDE,
    CODEX,
    PI,
    QODERCLI,
    XIAOO,
    SEMANTIC_ACTION_BOUNDARIES,
    OTEL_JSONL_ACTION_FILTER,
]


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run AcTrail v2 regression tests")
    add_common_arguments(parser)
    parser.add_argument(
        "--case",
        action="append",
        choices=[test.name for test in TESTS],
        dest="cases",
        help="case to run; repeatable (default: all)",
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
    arguments = parser.parse_args(argv)
    selected = [
        test for test in TESTS if not arguments.cases or test.name in arguments.cases
    ]
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
