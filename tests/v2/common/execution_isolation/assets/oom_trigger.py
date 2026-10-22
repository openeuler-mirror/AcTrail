#!/usr/bin/env python3

import os
import sys
import time

from constants import DEFAULTS


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: oom_trigger.py START_FILE")
    while not os.path.exists(sys.argv[1]):
        time.sleep(DEFAULTS.start_poll_seconds)
    allocations: list[bytearray] = []
    while True:
        block = bytearray(DEFAULTS.allocation_bytes)
        for offset in range(0, len(block), DEFAULTS.touch_stride_bytes):
            block[offset] = 1
        allocations.append(block)


if __name__ == "__main__":
    raise SystemExit(main())
