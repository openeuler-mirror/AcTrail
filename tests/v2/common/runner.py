from __future__ import annotations

import argparse
import os
import traceback
from contextlib import redirect_stderr, redirect_stdout
from dataclasses import dataclass
from pathlib import Path
from typing import Callable, TextIO

from .output import TestOutput, has_failure
from .test_case import TestCase, TestResult, TestStatus
from .testing_context import TestingContextSingleton


@dataclass(frozen=True)
class TestDefinition:
    name: str
    description: str
    build_case: Callable[[Path, Path], TestCase]


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
    os.environ.get("ACTRAIL_TEST_LOG_DIR", "/tmp/actrail-v2-regression")
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
        show_details=True,
    )


def run_selected(
    definitions: list[TestDefinition],
    repo: Path,
    bin_dir: Path,
    color_mode: str,
    log_dir: Path,
    *,
    show_details: bool,
) -> int:
    console = TestOutput(color_mode=color_mode)
    if os.geteuid() != 0:
        console.summary(
            "privilege check",
            TestResult(TestStatus.FAILED, "this eBPF E2E must run as root"),
        )
        return 1

    context = TestingContextSingleton()
    log_dir.mkdir(parents=True, exist_ok=True)
    failed = False
    for definition in definitions:
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
            context.output = case_output
            with redirect_stdout(runtime_stream), redirect_stderr(runtime_stream):
                try:
                    result = definition.build_case(repo, bin_dir).run(context)
                except Exception as error:
                    traceback.print_exc(file=runtime_stream)
                    result = TestResult(TestStatus.FAILED, str(error))
            TestOutput(color_mode="never", stream=log).result(
                definition.name,
                result,
            )
        if show_details:
            console.result(definition.name, result)
        else:
            console.progress_result(result)
        failed = failed or has_failure(result)
    return 1 if failed else 0
