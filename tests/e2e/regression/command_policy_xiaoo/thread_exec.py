from __future__ import annotations

import os
import sys
import threading


def exec_from_worker() -> None:
    task_id = threading.get_native_id()
    if task_id == os.getpid():
        raise RuntimeError("worker native task ID unexpectedly equals the process PID")
    os.execve(sys.argv[1], sys.argv[1:], os.environ)


def main() -> None:
    if len(sys.argv) < 2:
        raise RuntimeError("expected an executable and optional arguments")
    failures: list[BaseException] = []

    def worker() -> None:
        try:
            exec_from_worker()
        except BaseException as error:
            failures.append(error)

    thread = threading.Thread(target=worker, name="actrail-nonleader-exec")
    thread.start()
    thread.join()
    if failures:
        raise failures[0]
    raise RuntimeError("worker returned without replacing the process")


if __name__ == "__main__":
    main()
