from __future__ import annotations

import signal
from threading import Event
from types import FrameType


class ExitSignalWaiter:
    def __init__(self):
        self._exit_requested = Event()

    def install(self) -> None:
        signal.signal(signal.SIGINT, self._request_exit)
        signal.signal(signal.SIGTERM, self._request_exit)
        signal.signal(signal.SIGHUP, self._request_exit)

    def wait(self) -> None:
        self._exit_requested.wait()

    def _request_exit(
        self,
        _signal_number: int,
        _frame: FrameType | None,
    ) -> None:
        self._exit_requested.set()
