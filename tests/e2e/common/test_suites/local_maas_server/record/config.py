from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path


@dataclass(frozen=True, slots=True)
class RecordConfig:
    record: bool
    recordings_dir: Path

    def __post_init__(self) -> None:
        if not self.recordings_dir:
            raise ValueError("recordings_dir must be non-empty")
