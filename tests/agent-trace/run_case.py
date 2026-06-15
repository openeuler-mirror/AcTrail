#!/usr/bin/env python3
"""Run one real agent trace E2E case."""

from __future__ import annotations

import argparse
import os
import subprocess
import sys
from pathlib import Path


CASES = {
    "claude-code": "claude-code/run_e2e.py",
    "opencode-bun": "opencode-bun/run_e2e.py",
    "xiaoo-rustls": "xiaoo-rustls/run_e2e.py",
    "xiaoo-http-proxy": "xiaoo-http-proxy/run_e2e.py",
    "langgraph-openai": "langgraph-openai/run_e2e.py",
    "go-net-http": "go-net-http/run_e2e.py",
    "java-netty-tcnative": "java-netty-tcnative/run_e2e.py",
}


def main() -> int:
    args = parse_args()
    root = Path(__file__).resolve().parent
    selected = list(CASES) if args.case == "all" else [args.case]
    for case in selected:
        print(f"=== agent-trace case: {case} ===", flush=True)
        command = [
            sys.executable,
            str(root / CASES[case]),
            "--bin-dir",
            args.bin_dir,
        ]
        subprocess.run(command, cwd=root.parents[1], check=True)
    return 0


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("case", choices=[*CASES.keys(), "all"])
    parser.add_argument("--bin-dir", default=os.environ.get("ACTRAIL_BIN_DIR", "target/release"))
    return parser.parse_args()


if __name__ == "__main__":
    raise SystemExit(main())
