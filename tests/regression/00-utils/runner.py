"""Regression discovery, CLI, and terminal output."""

from __future__ import annotations

import argparse
import importlib.util
import os
import sys
import time
from pathlib import Path

from env_manager import EnvManager
from model import FAIL, PASS, SKIP, WARN, CaseResult, check_status_summary, set_color_mode
from reports import write_reports


DEFAULT_SUITE = "quick"
SUITES = (DEFAULT_SUITE, "agent", "payload", "enforcement", "full")
COLOR = {
    PASS: "\033[32m",
    FAIL: "\033[31m",
    SKIP: "\033[36m",
    WARN: "\033[33m",
    "path": "\033[36m",
    "reset": "\033[0m",
}


def main(regression_root: Path) -> int:
    args = parse_args(regression_root)
    cases = discover_cases(regression_root)
    if args.list_cases:
        print_case_list(cases, args)
        return 0
    output_dir = resolve_output_dir(args.output_dir)
    env = EnvManager.configure(regression_root, args.bin_dir, args.output_tail_chars, output_dir)
    selected = select_cases(cases, args)
    print("[*] initialize regression runner")
    print(f"    [*] repo: {env.repo_root}")
    print(f"    [*] output: {output_dir}")
    results: list[CaseResult] = []
    for index, case in enumerate(selected, start=1):
        result = run_case(index, case, env, args.color)
        results.append(result)
    markdown, machine = write_reports(output_dir, results)
    print("[*] reports")
    print(f"    - markdown: {color_path(markdown, args.color)}")
    print(f"    - json: {color_path(machine, args.color)}")
    return exit_code(results, args.strict)


def parse_args(regression_root: Path) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run AcTrail regression tests.")
    parser.add_argument("--suite", choices=SUITES, default=None, help=f"suite to run; defaults to {DEFAULT_SUITE}")
    parser.add_argument("--case", action="append", dest="cases", help="case id to run; repeatable")
    parser.add_argument(
        "--list",
        action="store_true",
        dest="list_cases",
        help="list available regression cases without running them",
    )
    parser.add_argument("--strict", action="store_true", help="treat skipped cases as failures")
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument(
        "--output-dir",
        default=str(Path("/tmp") / f"actrail-regression-{int(time.time())}"),
        help="directory for report.md, report.json, and per-case artifacts",
    )
    parser.add_argument("--color", choices=("auto", "always", "never"), default="auto")
    parser.add_argument(
        "--output-tail-chars",
        type=int,
        default=6000,
        help="characters retained from each command stdout/stderr in reports; use 0 for full output",
    )
    return parser.parse_args()


def discover_cases(regression_root: Path) -> list[object]:
    modules = []
    for path in sorted((regression_root / "cases").glob("*/test.py")):
        name = "actrail_regression_" + path.parent.name.replace("-", "_")
        spec = importlib.util.spec_from_file_location(name, path)
        if spec is None or spec.loader is None:
            raise RuntimeError(f"cannot load regression case {path}")
        module = importlib.util.module_from_spec(spec)
        sys.modules[name] = module
        spec.loader.exec_module(module)
        modules.append(module)
    return modules


def select_cases(cases: list[object], args: argparse.Namespace) -> list[object]:
    if args.cases:
        wanted = set(args.cases)
        selected = [case for case in cases if case.CASE_ID in wanted]
        missing = wanted - {case.CASE_ID for case in selected}
        if missing:
            raise RuntimeError(f"unknown regression case(s): {', '.join(sorted(missing))}")
        return selected
    suite = args.suite or DEFAULT_SUITE
    return [case for case in cases if suite in case.SUITES or suite == "full"]


def print_case_list(cases: list[object], args: argparse.Namespace) -> None:
    listed = select_cases(cases, args) if args.cases or args.suite else cases
    rows = [
        (
            str(case.CASE_ID),
            case_suites(case),
            str(case.TITLE),
        )
        for case in listed
    ]
    headers = ("CASE", "SUITES", "TITLE")
    widths = tuple(
        max(len(row[index]) for row in rows + [headers])
        for index in range(len(headers))
    )
    print("[*] available regression cases")
    print(f"    [*] suites: {', '.join(SUITES)}")
    print("    " + format_case_list_row(headers, widths))
    for row in rows:
        print("    " + format_case_list_row(row, widths))


def format_case_list_row(row: tuple[str, str, str], widths: tuple[int, int, int]) -> str:
    return "  ".join(value.ljust(width) for value, width in zip(row, widths))


def case_suites(case: object) -> str:
    declared = set(case.SUITES)
    ordered = [suite for suite in SUITES if suite in declared]
    extras = sorted(declared - set(SUITES))
    return ",".join(ordered + extras)


def run_case(index: int, case: object, env: EnvManager, color_mode: str) -> CaseResult:
    print(f"[*] Test {index}: {case.TITLE}")
    set_color_mode(color_mode)
    started = time.monotonic()
    try:
        result = case.run(env)
    except Exception as error:
        result = CaseResult(case.CASE_ID, case.TITLE, FAIL, 0.0)
        result.add_check("unhandled exception", FAIL, str(error))
    result.duration_seconds = time.monotonic() - started
    print_case_result(result, color_mode)
    return result


def print_case_result(result: CaseResult, color_mode: str) -> None:
    print(f"    ... {color_check_summary(result, color_mode)}")


def color_check_summary(result: CaseResult, color_mode: str) -> str:
    if not should_color(color_mode):
        return check_status_summary(result)
    raw_parts = check_status_summary(result).split()
    colored_parts = []
    for part in raw_parts:
        status, count = part.split("=", 1)
        colored_parts.append(f"{color_status(status.lower(), color_mode)}={count}")
    return " ".join(colored_parts)


def color_status(status: str, color_mode: str) -> str:
    label = status.upper()
    if not should_color(color_mode):
        return label
    return f"{COLOR.get(status, '')}{label}{COLOR['reset']}"


def color_path(path: Path, color_mode: str) -> str:
    raw = str(path)
    if not should_color(color_mode):
        return raw
    return f"{COLOR['path']}{raw}{COLOR['reset']}"


def should_color(color_mode: str) -> bool:
    if color_mode == "never":
        return False
    if color_mode == "auto" and not sys.stdout.isatty():
        return False
    return True


def resolve_output_dir(raw: str) -> Path:
    return Path(raw).resolve()


def exit_code(results: list[CaseResult], strict: bool) -> int:
    if any(result.status == FAIL for result in results):
        return 1
    if strict and any(result.status == SKIP for result in results):
        return 1
    return 0
