#!/usr/bin/env python3

import os
import sys
import time

from constants import CASE_CONSTANTS


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: oom_trigger.py START_FILE")
    start_file = sys.argv[1]
    while not os.path.exists(start_file):
        time.sleep(CASE_CONSTANTS.oom_start_poll_seconds)
    allocations: list[bytearray] = []
    while True:
        block = bytearray(CASE_CONSTANTS.oom_allocation_bytes)
        for offset in range(0, len(block), CASE_CONSTANTS.oom_page_stride_bytes):
            block[offset] = 1
        allocations.append(block)


if __name__ == "__main__":
    raise SystemExit(main())
