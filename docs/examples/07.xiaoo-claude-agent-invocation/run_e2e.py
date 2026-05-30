#!/usr/bin/env python3
"""Run the docs xiaoO -> Claude Code agent invocation E2E."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    runner = repo / "tests/process/agent-invocation/run_e2e.py"
    command = [
        sys.executable,
        str(runner),
        "--bin-dir",
        args.bin_dir,
        "--config",
        str(args.config),
        "--workload-config",
        str(args.workload_config),
    ]
    return subprocess.run(command, cwd=repo, check=False).returncode


def parse_args() -> argparse.Namespace:
    example_dir = Path(__file__).resolve().parent
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    parser.add_argument("--config", default=str(example_dir / "operator.conf"))
    parser.add_argument("--workload-config", default=str(example_dir / "workload.conf"))
    return parser.parse_args()


if __name__ == "__main__":
    raise SystemExit(main())
