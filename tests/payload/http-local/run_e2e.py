#!/usr/bin/env python3
"""Run the public HTTP socket payload E2E with the regression config."""

from __future__ import annotations

import subprocess
import sys
from pathlib import Path


def main() -> int:
    repo = Path(__file__).resolve().parents[3]
    example_runner = repo / "docs/examples/05.http-payload-unified/run_e2e.py"
    test_dir = Path(__file__).resolve().parent
    command = [
        sys.executable,
        str(example_runner),
        "--config",
        str(test_dir / "operator.conf"),
        "--workload",
        str(test_dir / "workload.py"),
    ]
    command.extend(sys.argv[1:])
    return subprocess.call(command)


if __name__ == "__main__":
    raise SystemExit(main())
