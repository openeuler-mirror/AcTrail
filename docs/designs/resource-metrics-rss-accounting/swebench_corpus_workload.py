#!/usr/bin/env python3
"""Run parallel analytics over resource-oracle's staged SWE-bench corpus."""

import argparse
import json
import multiprocessing as mp
import os
import time
from pathlib import Path

import pandas as pd
import pyarrow.parquet as pq


ROWS = []
TASKS = []


def worker(deadline, worker_index, worker_count):
    checksum = 0
    while time.monotonic() < deadline:
        frame = pd.DataFrame.from_records(ROWS)
        summary = frame.groupby("benchmark_id", sort=True)["mem_peak_mb"].agg(
            ["count", "mean", "max"]
        )
        checksum ^= hash(tuple(summary["max"].fillna(0).astype(int)))
        checksum ^= sum(
            len(statement)
            for index, statement in enumerate(TASKS)
            if index % worker_count == worker_index
        )
    return checksum


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--corpus", required=True)
    parser.add_argument("--parquet", required=True)
    parser.add_argument("--ready-file", required=True)
    parser.add_argument("--start-file", required=True)
    parser.add_argument("--seconds", type=float, default=12.0)
    parser.add_argument("--workers", type=int, default=4)
    args = parser.parse_args()

    if args.seconds <= 0 or args.workers <= 0:
        raise SystemExit("seconds and workers must be positive")

    global ROWS, TASKS
    with open(args.corpus, encoding="utf-8") as handle:
        raw_rows = [json.loads(line) for line in handle]
    ROWS = [
        {
            "benchmark_id": row["benchmark_id"],
            "mem_peak_mb": (row.get("measured_usage") or {}).get("mem_peak_mb"),
        }
        for row in raw_rows
    ]
    TASKS = pq.read_table(args.parquet, columns=["problem_statement"])[
        "problem_statement"
    ].to_pylist()

    context = mp.get_context("fork")
    with context.Pool(args.workers) as pool:
        Path(args.ready_file).write_text(str(os.getpid()), encoding="ascii")
        start_file = Path(args.start_file)
        while not start_file.exists():
            time.sleep(0.02)
        deadline = time.monotonic() + args.seconds
        checksums = pool.starmap(
            worker,
            [
                (deadline, worker_index, args.workers)
                for worker_index in range(args.workers)
            ],
        )

    print(
        json.dumps(
            {
                "rows": len(ROWS),
                "tasks": len(TASKS),
                "workers": args.workers,
                "checksums": checksums,
            },
            sort_keys=True,
        )
    )


if __name__ == "__main__":
    main()
