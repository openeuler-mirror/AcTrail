#!/usr/bin/env python3
"""Run the real-agent Fast Open regression in a temporary network namespace."""

import ctypes
import os
from pathlib import Path
import subprocess
import sys


class FastOpenRegression:
    CLONE_NEWNET = 0x40000000

    def run(self):
        namespace = Path("/proc/self/ns/net")
        original = namespace.stat()
        libc = ctypes.CDLL(None, use_errno=True)
        libc.unshare.argtypes = [ctypes.c_int]
        libc.unshare.restype = ctypes.c_int
        if libc.unshare(self.CLONE_NEWNET) != 0:
            code = ctypes.get_errno()
            raise OSError(code, os.strerror(code))
        current = namespace.stat()
        if (current.st_dev, current.st_ino) == (original.st_dev, original.st_ino):
            raise RuntimeError("Fast Open test requires its own network namespace")
        # Every configuration change below applies only to this temporary netns.
        subprocess.run(["ip", "link", "set", "lo", "up"], check=True)
        subprocess.run(["sysctl", "-w", "net.ipv4.tcp_fastopen=3"], check=True)
        print(f"Fast Open test netns={current.st_ino}; original netns={original.st_ino}", flush=True)
        environment = os.environ.copy()
        environment.update(LSM_ACCESS_CONTROL_XIAOO_E2E_BACKENDS="lsm",
                           LSM_ACCESS_CONTROL_XIAOO_E2E_CONTINUED="1",
                           LSM_ACCESS_CONTROL_XIAOO_E2E_FASTOPEN="1")
        entry = Path(__file__).resolve().parents[2] / "run_e2e.py"
        return subprocess.run([sys.executable, str(entry), "--color", "never", *sys.argv[1:]],
                              env=environment).returncode


if __name__ == "__main__":
    raise SystemExit(FastOpenRegression().run())
