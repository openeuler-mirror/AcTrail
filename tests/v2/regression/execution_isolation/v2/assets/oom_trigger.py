#!/usr/bin/env python3

import os
import sys
import time


def main() -> int:
    if len(sys.argv) != 2:
        raise SystemExit("usage: oom_trigger.py START_FILE")
    start_file = sys.argv[1]
    while not os.path.exists(start_file):
        time.sleep(0.01)
    allocations: list[bytearray] = []
    while True:
        block = bytearray(1024 * 1024)
        for offset in range(0, len(block), 4096):
            block[offset] = 1
        allocations.append(block)


if __name__ == "__main__":
    raise SystemExit(main())
