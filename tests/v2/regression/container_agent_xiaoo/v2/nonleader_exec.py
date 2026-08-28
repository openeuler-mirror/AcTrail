#!/usr/bin/env python3
"""Replace a multithreaded process from a non-leader worker thread."""

from __future__ import annotations

import os
import sys
import threading
import time


def exec_from_worker(marker_path: str) -> None:
    os.execv(
        "/bin/sh",
        [
            "/bin/sh",
            "-c",
            'printf "%s\\n" "$1" >"$2"',
            "actrail-nonleader-exec",
            "NONLEADER_EXEC_COMPLETE",
            marker_path,
        ],
    )


def main() -> int:
    if len(sys.argv) != 2:
        raise RuntimeError("nonleader exec fixture requires one marker path")
    worker = threading.Thread(target=exec_from_worker, args=(sys.argv[1],))
    worker.start()
    while worker.is_alive():
        time.sleep(0.05)
    raise RuntimeError("worker-thread exec unexpectedly returned")


if __name__ == "__main__":
    raise SystemExit(main())
