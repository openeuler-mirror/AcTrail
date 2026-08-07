from __future__ import annotations

import argparse
import os
import shutil
import traceback
from contextlib import redirect_stderr, redirect_stdout
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, TextIO

from ..core import (
    CaseProgressReporter,
    TestCase,
    TestCaseInputs,
    TestOutput,
    TestResult,
    TestStatus,
    effective_status,
    has_failure,
)
from .environment_probe import EnvironmentProbe
from .testing_context import TestingContextSingleton


@dataclass(frozen=True)
class TestDefinition:
    name: str
    description: str
    build_case: Callable[[TestCaseInputs], TestCase]
    skip_if_skipped: tuple[str, ...] = ()


class Tee:
    def __init__(self, *streams: TextIO):
        self._streams = streams

    def write(self, value: str) -> int:
        for stream in self._streams:
            stream.write(value)
        return len(value)

    def flush(self) -> None:
        for stream in self._streams:
            stream.flush()


DEFAULT_LOG_DIR = Path(
    os.environ.get("ACTRAIL_TEST_LOG_DIR", "/tmp/actrail-regression/logs")
)
DEFAULT_WORK_ROOT = Path(
    os.environ.get("ACTRAIL_TEST_WORK_ROOT", "/tmp/actrail-regression")
)
DEFAULT_LOCK_PATH = Path(
    os.environ.get(
        "ACTRAIL_TEST_LOCK_PATH",
        "/run/lock/actrail-v2-regression.lock",
    )
)


def add_common_arguments(parser: argparse.ArgumentParser) -> None:
    parser.add_argument(
        "--bin-dir",
        type=Path,
        default=Path(os.environ.get("ACTRAIL_BIN_DIR", "target/release")),
        help="directory containing release AcTrail binaries",
    )
    parser.add_argument(
        "--color",
        choices=("auto", "always", "never"),
        default="auto",
        help="colored result symbols (default: auto)",
    )
    parser.add_argument(
        "--log-dir",
        type=Path,
        default=DEFAULT_LOG_DIR,
        help=f"directory for per-case logs (default: {DEFAULT_LOG_DIR})",
    )
    parser.add_argument(
        "--work-root",
        type=Path,
        default=DEFAULT_WORK_ROOT,
        help=f"root directory for isolated per-case state (default: {DEFAULT_WORK_ROOT})",
    )
    parser.add_argument(
        "--cleanup",
        action=argparse.BooleanOptionalAction,
        default=True,
        help="run case cleanup hooks (default: enabled; use --no-cleanup to preserve state)",
    )
    parser.add_argument(
        "--lock-path",
        type=Path,
        default=DEFAULT_LOCK_PATH,
        help=(
            "global regression lock path "
            f"(default: {DEFAULT_LOCK_PATH}; env: ACTRAIL_TEST_LOCK_PATH)"
        ),
    )
    parser.add_argument(
        "--lock-timeout-seconds",
        type=float,
        default=os.environ.get("ACTRAIL_TEST_LOCK_TIMEOUT_SECONDS", "900"),
        help=(
            "maximum total wait for another regression thread or process "
            "(default: 900; env: ACTRAIL_TEST_LOCK_TIMEOUT_SECONDS)"
        ),
    )
    parser.add_argument(
        "--lock-poll-seconds",
        type=float,
        default=os.environ.get("ACTRAIL_TEST_LOCK_POLL_SECONDS", "1"),
        help=(
            "regression lock polling interval "
            "(default: 1; env: ACTRAIL_TEST_LOCK_POLL_SECONDS)"
        ),
    )


def run_one(
    definition: TestDefinition,
    repo: Path,
    argv: list[str] | None = None,
) -> int:
    parser = argparse.ArgumentParser(description=definition.description)
    add_common_arguments(parser)
    arguments = parser.parse_args(argv)
    return run_selected(
        [definition],
        repo,
        arguments.bin_dir,
        arguments.color,
        arguments.log_dir,
        arguments.work_root,
        show_details=True,
        cleanup_cases=arguments.cleanup,
        lock_path=arguments.lock_path,
        lock_timeout_seconds=arguments.lock_timeout_seconds,
        lock_poll_seconds=arguments.lock_poll_seconds,
    )


def run_selected(
    definitions: list[TestDefinition],
    repo: Path,
    bin_dir: Path,
    color_mode: str,
    log_dir: Path,
    work_root: Path,
    *,
    show_details: bool,
    cleanup_cases: bool = True,
    fail_fast: bool = False,
    lock_path: Path = DEFAULT_LOCK_PATH,
    lock_timeout_seconds: float = 900,
    lock_poll_seconds: float = 1,
) -> int:
    console = TestOutput(color_mode=color_mode)
    if os.geteuid() != 0:
        console.summary(
            "privilege check",
            TestResult(TestStatus.FAILED, "this eBPF E2E must run as root"),
        )
        return 1

    try:
        context = TestingContextSingleton(
            lock_path=lock_path,
            lock_timeout_seconds=lock_timeout_seconds,
            lock_poll_seconds=lock_poll_seconds,
            output=console,
        )
    except (OSError, TimeoutError, ValueError) as error:
        console.result(
            "regression_lock",
            TestResult(TestStatus.FAILED, str(error)),
        )
        return 1
    try:
        EnvironmentProbe(console).print_summary()
    except Exception as error:
        console.line(f"environment_probe=failed: {error}")
    try:
        try:
            context.prepare_release(repo)
        except RuntimeError as error:
            console.result(
                "release_install",
                TestResult(TestStatus.FAILED, str(error)),
            )
            return 1
        work_root = work_root.resolve()
        _validate_work_root(work_root, repo.resolve())
        log_dir.mkdir(parents=True, exist_ok=True)
        failed = False
        completed_results: dict[str, TestResult] = {}
        for definition in definitions:
            case_work_dir = work_root / definition.name
            if show_details:
                console.heading(f"▶ {definition.name}")
            else:
                console.progress(definition.name)
            log_path = log_dir / f"{definition.name}.log"
            with log_path.open("w", encoding="utf-8") as log:
                log.write(f"test: {definition.name}\n")
                log.write(f"description: {definition.description}\n")
                runtime_stream = Tee(log, console.stream) if show_details else log
                case_output = TestOutput(color_mode="never", stream=runtime_stream)
                log_output = TestOutput(color_mode="never", stream=log)
                progress_reporter = CaseProgressReporter(
                    console,
                    log_output,
                    detailed=show_details,
                )
                with context.output_scope(
                    case_output
                ), context.progress_scope(
                    progress_reporter
                ), redirect_stdout(
                    runtime_stream
                ), redirect_stderr(
                    runtime_stream
                ):
                    case: TestCase | None = None
                    result: TestResult | None = None
                    try:
                        try:
                            result = _dependency_skip_result(
                                definition,
                                completed_results,
                            )
                            if result is None:
                                _prepare_case_work_dir(case_work_dir, work_root)
                                case = definition.build_case(
                                    TestCaseInputs(repo, bin_dir, case_work_dir)
                                )
                                result = case.run(context)
                        except Exception as error:
                            traceback.print_exc(file=runtime_stream)
                            result = TestResult(TestStatus.FAILED, str(error))
                    finally:
                        if cleanup_cases and case is not None:
                            try:
                                context.report_progress(
                                    "cleanup",
                                    "running cleanup hooks",
                                )
                                cleanup_result = case.cleanup(context)
                            except Exception as error:
                                traceback.print_exc(file=runtime_stream)
                                cleanup_result = TestResult(
                                    TestStatus.FAILED, str(error)
                                )
                            if (
                                result is not None
                                and cleanup_result is not None
                                and has_failure(cleanup_result)
                            ):
                                result = TestResult(
                                    TestStatus.COMPOSITE,
                                    result.message,
                                    {
                                        "test": result,
                                        "cleanup": cleanup_result,
                                    },
                                )
                        if cleanup_cases:
                            try:
                                _remove_case_work_dir(case_work_dir, work_root)
                            except Exception as error:
                                traceback.print_exc(file=runtime_stream)
                                if result is not None:
                                    result = TestResult(
                                        TestStatus.COMPOSITE,
                                        result.message,
                                        {
                                            "test": result,
                                            "workspace_cleanup": TestResult(
                                                TestStatus.FAILED,
                                                str(error),
                                            ),
                                        },
                                    )
                    if result is None:
                        raise RuntimeError(
                            f"test case returned no result: {definition.name}"
                        )
                TestOutput(color_mode="never", stream=log).result(
                    definition.name,
                    result,
                )
            if cleanup_cases:
                try:
                    if log_path.exists():
                        log_path.unlink()
                except Exception as error:
                    result = TestResult(
                        TestStatus.COMPOSITE,
                        result.message,
                        {
                            "test": result,
                            "log_cleanup": TestResult(
                                TestStatus.FAILED,
                                str(error),
                            ),
                        },
                    )
            if show_details:
                console.result(definition.name, result)
            else:
                console.progress_result(result)
                if cleanup_cases and has_failure(result):
                    console.result(definition.name, result)
            case_failed = has_failure(result)
            completed_results[definition.name] = result
            failed = failed or case_failed
            if fail_fast and case_failed:
                break
        if cleanup_cases:
            for directory in (log_dir, work_root):
                try:
                    directory.rmdir()
                except OSError:
                    pass
        return 1 if failed else 0
    finally:
        context.close()


def _dependency_skip_result(
    definition: TestDefinition,
    completed_results: dict[str, TestResult],
) -> TestResult | None:
    skipped = [
        name
        for name in definition.skip_if_skipped
        if name in completed_results
        and effective_status(completed_results[name]) is TestStatus.SKIPPED
    ]
    if not skipped:
        return None
    return TestResult(
        TestStatus.SKIPPED,
        "prerequisite case skipped: " + ", ".join(skipped),
    )


def _remove_case_work_dir(case_work_dir: Path, work_root: Path) -> None:
    _validate_case_work_dir(case_work_dir, work_root)
    if case_work_dir.exists():
        shutil.rmtree(case_work_dir)
    try:
        work_root.rmdir()
    except OSError:
        pass


def _prepare_case_work_dir(case_work_dir: Path, work_root: Path) -> None:
    _validate_case_work_dir(case_work_dir, work_root)
    if case_work_dir.exists():
        shutil.rmtree(case_work_dir)
    case_work_dir.mkdir(parents=True)


def _validate_case_work_dir(case_work_dir: Path, work_root: Path) -> None:
    if case_work_dir.parent != work_root or case_work_dir == work_root:
        raise RuntimeError(
            f"unsafe case work directory: {case_work_dir}"
        )


def _validate_work_root(work_root: Path, repo: Path) -> None:
    forbidden = {Path("/"), Path.home().resolve(), repo}
    if work_root in forbidden:
        raise RuntimeError(f"refusing unsafe test work root: {work_root}")
