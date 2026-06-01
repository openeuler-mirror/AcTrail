#!/usr/bin/env python3
"""Run the public HTTP socket payload workload from the regression tree."""

from __future__ import annotations

import runpy
from pathlib import Path


if __name__ == "__main__":
    repo = Path(__file__).resolve().parents[3]
    runpy.run_path(
        str(repo / "docs/examples/05.http-payload-unified/workload.py"),
        run_name="__main__",
    )
