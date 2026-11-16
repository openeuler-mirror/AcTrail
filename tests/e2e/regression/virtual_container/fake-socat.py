#!/usr/bin/env python3
"""Record the public socat invocation used by bridge contract tests."""

from __future__ import annotations

import os
import signal
import sys
from pathlib import Path


log_path = os.environ.get("FAKE_SOCAT_ARGS_LOG")
if not log_path:
    print("FAKE_SOCAT_ARGS_LOG is required", file=sys.stderr)
    raise SystemExit(2)

with Path(log_path).open("a", encoding="utf-8") as handle:
    handle.write("\t".join(sys.argv[1:]) + "\n")

if os.environ.get("FAKE_SOCAT_HOLD") == "1":
    while True:
        signal.pause()
