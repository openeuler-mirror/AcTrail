#!/usr/bin/env python3
"""Agent trace case for real Claude Code LLM request capture."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


def main() -> int:
    args = parse_args()
    repo = Path(__file__).resolve().parents[3]
    runner = repo / "tests/payload/claude-code/run_e2e.py"
    command = [
        sys.executable,
        str(runner),
        "--bin-dir",
        args.bin_dir,
    ]
    return subprocess.run(command, cwd=repo, check=False).returncode


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    return parser.parse_args()


if __name__ == "__main__":
    raise SystemExit(main())
