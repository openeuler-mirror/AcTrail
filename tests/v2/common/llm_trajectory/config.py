from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path

from tests.v2.common.core import CommonTestConfig


@dataclass(frozen=True)
class TrajectoryTestConfig(CommonTestConfig):
    operator_config: Path
    web_host: str
    web_port: int
    plugin_package: str
    plugin_instance: str
    request_content_max_bytes: int
