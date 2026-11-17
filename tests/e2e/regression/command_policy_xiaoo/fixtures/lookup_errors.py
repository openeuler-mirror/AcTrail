"""Exercise actual failed exec syscalls inside the governed trace."""

from __future__ import annotations

import errno
import os
import sys


class NativeLookupVerification:
    def run(self) -> None:
        if len(sys.argv) != 3:
            raise RuntimeError("expected a missing executable and a non-directory PATH candidate")
        observed = []
        for path, expected in zip(sys.argv[1:], (errno.ENOENT, errno.ENOTDIR), strict=True):
            try:
                os.execve(path, [path], os.environ)
            except OSError as error:
                if error.errno != expected:
                    raise AssertionError(f"{path}: expected errno {expected}, received {error.errno}") from error
                observed.append(error.errno)
        print("ACTRAIL_NATIVE_ERRNO_OK", *observed)


if __name__ == "__main__":
    NativeLookupVerification().run()
