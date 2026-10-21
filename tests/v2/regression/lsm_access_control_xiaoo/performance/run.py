#!/usr/bin/env python3
"""Measure real Xiaoo TCP policy workloads with seccomp and BPF LSM.

Uses existing release binaries and the shared regression lock. The default
policy source is the official WASM publisher. Results are written to local/.
"""

from __future__ import annotations

import argparse
import sys
from pathlib import Path

if __package__:
    from .constants import CASE_CONSTANTS
else:
    from constants import CASE_CONSTANTS


sys.path.insert(0, str(Path(__file__).resolve().parents[5]))

from tests.v2.regression.lsm_access_control_xiaoo.performance.benchmark import Benchmark
from tests.v2.regression.lsm_access_control_xiaoo.performance.paths import OUTPUT_DIR, REPO
from tests.v2.regression.lsm_access_control_xiaoo.performance.report import Report
from tests.v2.regression.lsm_access_control_xiaoo.performance.worker import Worker


class CommandLine:
    @staticmethod
    def run() -> None:
        parser = argparse.ArgumentParser(description=__doc__)
        modes = parser.add_subparsers(dest="mode", required=True)
        benchmark = modes.add_parser("run", help="run both backends with existing release binaries")
        worker = modes.add_parser("worker", help=argparse.SUPPRESS)
        report = modes.add_parser("report", help="summarize existing measurements")
        report.add_argument("--input", type=Path, default=OUTPUT_DIR.joinpath(CASE_CONSTANTS.report_input_filename))
        report.add_argument("--output", type=Path, default=OUTPUT_DIR.joinpath(CASE_CONSTANTS.report_output_filename))
        for mode in (benchmark, worker):
            mode.add_argument("--count", type=int, default=CASE_CONSTANTS.count)
            mode.add_argument("--rounds", type=int, default=CASE_CONSTANTS.rounds)
            mode.add_argument("--warmup", type=int, default=CASE_CONSTANTS.warmup)
            mode.add_argument("--timeout", type=float, default=CASE_CONSTANTS.timeout)
            mode.add_argument("--xiaoo", type=Path)
        benchmark.add_argument("--bin-dir", type=Path, default=REPO / CASE_CONSTANTS.bin_dir)
        benchmark.add_argument("--policy-source", choices=("static", "wasm"), default=CASE_CONSTANTS.policy_source)
        benchmark.add_argument("--output", type=Path)
        benchmark.add_argument("--backends", nargs=2, choices=("seccomp", "lsm"), default=CASE_CONSTANTS.backends.copy())
        benchmark.add_argument("--command-timeout", type=int, default=CASE_CONSTANTS.command_timeout)
        benchmark.add_argument("--launch-timeout", type=int, default=CASE_CONSTANTS.launch_timeout)
        benchmark.add_argument("--lock-timeout", type=float, default=CASE_CONSTANTS.lock_timeout)
        benchmark.add_argument("--peer-poll-seconds", type=float, default=CASE_CONSTANTS.peer_poll_seconds)
        benchmark.add_argument("--backlog", type=int, default=CASE_CONSTANTS.backlog)
        benchmark.add_argument("--keep-work", action="store_true", default=CASE_CONSTANTS.keep_work)
        worker.add_argument("--backend", choices=("seccomp", "lsm"), required=True)
        worker.add_argument("--allow-port", type=int, required=True)
        worker.add_argument("--deny-port", type=int, required=True)
        worker.add_argument("--daemon-pid", type=Path, required=True)
        worker.add_argument("--results", type=Path, required=True)
        args = parser.parse_args()
        if args.mode == "report":
            Report(args.input, args.output).run()
            return
        if args.count < 1 or args.rounds < 3 or args.warmup < 1 or args.timeout <= 0:
            parser.error("count/warmup/timeout must be positive and rounds must be at least 3")
        if args.mode == "run":
            if args.output is None:
                args.output = OUTPUT_DIR.joinpath(CASE_CONSTANTS.report_input_filename if args.policy_source == "wasm" else CASE_CONSTANTS.static_report_filename)
            if set(args.backends) != {"seccomp", "lsm"}:
                parser.error("--backends must contain seccomp and lsm exactly once")
            if min(args.command_timeout, args.launch_timeout, args.lock_timeout,
                   args.peer_poll_seconds, args.backlog) <= 0:
                parser.error("timeouts, peer poll interval, and backlog must be positive")
            Benchmark(args).run()
        else:
            if args.xiaoo is None:
                parser.error("worker requires --xiaoo")
            Worker(args).run()


if __name__ == "__main__":
    CommandLine.run()
