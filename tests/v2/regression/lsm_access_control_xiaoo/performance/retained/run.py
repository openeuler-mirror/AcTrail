#!/usr/bin/env python3
"""Benchmark retained FD/socket operations under real Xiaoo with fresh defaults."""

import argparse
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parents[6]
sys.path.insert(0, str(REPO))
from tests.v2.regression.lsm_access_control_xiaoo.performance.constants import CASE_CONSTANTS
from tests.v2.regression.lsm_access_control_xiaoo.performance.retained.benchmark import RetainedBenchmark
from tests.v2.regression.lsm_access_control_xiaoo.performance.retained.worker import RetainedWorker


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=("run", "worker"))
    parser.add_argument("--count", type=int, default=CASE_CONSTANTS.count)
    parser.add_argument("--rounds", type=int, default=CASE_CONSTANTS.rounds)
    parser.add_argument("--warmup", type=int, default=CASE_CONSTANTS.warmup)
    parser.add_argument("--timeout", type=float, default=CASE_CONSTANTS.timeout)
    parser.add_argument("--command-timeout", type=int, default=CASE_CONSTANTS.command_timeout)
    parser.add_argument("--launch-timeout", type=int, default=CASE_CONSTANTS.launch_timeout)
    parser.add_argument("--xiaoo", type=Path)
    parser.add_argument("--bin-dir", type=Path, default=REPO / "target/release")
    parser.add_argument("--output", type=Path, default=REPO / "local/lsm_access_control/retained.json")
    parser.add_argument("--order", nargs="+", choices=("off", "lsm"), default=["off", "lsm", "lsm", "off"])
    parser.add_argument("--backend", choices=("off", "lsm"))
    parser.add_argument("--file", type=Path)
    parser.add_argument("--port", type=int)
    parser.add_argument("--daemon-pid", type=Path)
    parser.add_argument("--results", type=Path)
    args = parser.parse_args()
    if min(args.count, args.rounds, args.warmup, args.timeout) <= 0:
        parser.error("count, rounds, warmup and timeout must be positive")
    if args.mode == "worker":
        RetainedWorker(args).run()
    else:
        RetainedBenchmark(args).run()


if __name__ == "__main__":
    main()
