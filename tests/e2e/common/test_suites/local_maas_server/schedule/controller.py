from __future__ import annotations

import time
from typing import Iterable, Iterator

from protocol.interface import ProtocolFrame

from .config import ScheduleConfig


class ScheduleController:
    def __init__(self, config: ScheduleConfig):
        self._config = config

    def apply(
        self, frames: Iterable[ProtocolFrame]
    ) -> Iterator[ProtocolFrame]:
        first = True
        for frame in frames:
            self._sleep(
                self._config.ttft_seconds
                if first
                else self._config.tpot_seconds
            )
            first = False
            yield frame

    @staticmethod
    def _sleep(seconds: float) -> None:
        if seconds > 0:
            time.sleep(seconds)
