#!/usr/bin/env python3
"""Run all end-to-end regression cases through the common test runner."""

from __future__ import annotations

import argparse
import sys
from pathlib import Path


REPO = Path(__file__).resolve().parents[3]
sys.path.insert(0, str(REPO))

from tests.e2e.regression.constants import DEFAULTS

from tests.e2e.common.core import TestOutput
from tests.e2e.common.runner import TestCaseFactory, add_common_arguments, run_selected
from tests.e2e.regression.local_profile import (  # noqa: E402
    load_local_test_profile,
)
def main(argv: list[str] | None = None) -> int:
    tests = TestCaseFactory.discover(Path(__file__).parent)
    effective_argv = list(sys.argv[1:] if argv is None else argv)
    bootstrap_parser = argparse.ArgumentParser(add_help=False)
    bootstrap_parser.add_argument("--profile", type=Path)
    bootstrap, _ = bootstrap_parser.parse_known_args(effective_argv)
    try:
        loaded_profile = (
            load_local_test_profile(REPO, bootstrap.profile)
            if bootstrap.profile is not None
            else None
        )
    except ValueError as error:
        bootstrap_parser.error(str(error))

    parser = argparse.ArgumentParser(description="Run AcTrail end-to-end regression tests")
    add_common_arguments(parser)
    parser.add_argument(
        "--profile",
        type=Path,
        help="explicit machine-local test profile (none loaded by default)",
    )
    parser.add_argument(
        "--case",
        action="append",
        choices=[test.case_id for test in tests],
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
        selected = [test for test in tests if test.case_id in requested]
    elif arguments.list_cases:
        selected = tests
    else:
        selected = [test for test in tests if test.default_enabled]
    if loaded_profile is not None:
        TestOutput(color_mode=arguments.color).line(
            f"test_profile={loaded_profile}"
        )
    if arguments.list_cases:
        output = TestOutput(color_mode=arguments.color)
        for test in selected:
            output.line(f"{test.case_id}: {test.description}")
        return 0
    return run_selected(
        selected,
        REPO,
        arguments.bin_dir,
        arguments.color,
        arguments.log_dir,
        arguments.work_root,
        show_details=DEFAULTS.show_details,
        cleanup_cases=arguments.cleanup,
        fail_fast=arguments.fail_fast,
        lock_path=arguments.lock_path,
        lock_timeout_seconds=arguments.lock_timeout_seconds,
        lock_poll_seconds=arguments.lock_poll_seconds,
    )


if __name__ == "__main__":
    raise SystemExit(main())
