#!/usr/bin/env python3
"""Fork a memory-using descendant and record pre-exec cgroup membership."""

from __future__ import annotations

import ctypes
import os
import signal
import sys
import time
from pathlib import Path

from constants import CASE_CONSTANTS


output = Path(sys.argv[1])
suffix = sys.argv[2]
child_seconds = float(sys.argv[3])
parent_seconds = float(sys.argv[4])


def membership() -> str:
    return Path("/proc/self/cgroup").read_text(encoding="utf-8").strip()


parent_memory = bytearray(CASE_CONSTANTS.parent_memory_bytes)
parent_memory[::CASE_CONSTANTS.memory_touch_stride_bytes] = b"p" * (len(parent_memory) // CASE_CONSTANTS.memory_touch_stride_bytes)
(output / f"parent-cgroup-{suffix}").write_text(membership(), encoding="utf-8")

child = os.fork()
if child == 0:
    # A descendant must survive both the launch root and a daemon service crash.
    ctypes.CDLL(None).prctl(1, 0, 0, 0, 0)
    signal.signal(signal.SIGHUP, signal.SIG_IGN)
    signal.signal(signal.SIGTERM, signal.SIG_IGN)
    child_memory = bytearray(CASE_CONSTANTS.child_memory_bytes)
    child_memory[::CASE_CONSTANTS.memory_touch_stride_bytes] = b"c" * (len(child_memory) // CASE_CONSTANTS.memory_touch_stride_bytes)
    (output / f"child-pid-{suffix}").write_text(
        str(os.getpid()), encoding="utf-8"
    )
    (output / f"child-cgroup-{suffix}").write_text(
        membership(), encoding="utf-8"
    )
    time.sleep(child_seconds)
    (output / f"child-done-{suffix}").write_text("done\n", encoding="utf-8")
    os._exit(0)

(output / f"parent-pid-{suffix}").write_text(str(os.getpid()), encoding="utf-8")
time.sleep(parent_seconds)
